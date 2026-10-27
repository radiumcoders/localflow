//! localflow's cleanup model, run in this process with llama.cpp: no server,
//! nothing else to install.
//!
//! llama.cpp brings a newer ggml than the one whisper.cpp links into the
//! binary, and two ggml in one binary clash. So llama.cpp lives in a shared
//! library of its own (native/), with ggml hidden inside it, that is embedded
//! in the binary and written out next to the models the first time it's
//! needed. GPU through Vulkan (feature "vulkan") or Metal (macOS), else CPU.
//!
//! ```no_run
//! let mut model = localflow_llm::Model::load("localflow-clean.gguf".as_ref(), true, 4096)?;
//! let (text, cut_off) = model.complete("<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n", 64)?;
//! # Ok::<(), String>(())
//! ```

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/llm.zst"));

type Load = unsafe extern "C" fn(*const c_char, c_int, c_int, *mut *mut c_char) -> *mut c_void;
type Complete = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_int, *mut *mut c_char) -> *mut c_char;
type FreeString = unsafe extern "C" fn(*mut c_char);
type Free = unsafe extern "C" fn(*mut c_void);

struct Lib {
    load: Load,
    complete: Complete,
    free_string: FreeString,
    free: Free,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// ~/.local/share/localflow/lib, ~/Library/Application Support/localflow/lib,
/// %LOCALAPPDATA%\localflow\lib: beside the models.
fn lib_dir() -> PathBuf {
    let home = || PathBuf::from(env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default());
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Local"))
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else {
        env("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    };
    base.join("localflow").join("lib")
}

/// The embedded library on disk: written once per build of it, under a name
/// that changes with its contents; older ones are removed.
fn unpack() -> Result<PathBuf, String> {
    let mut hash = DefaultHasher::new();
    hash.write(PACKED);
    let (prefix, ext) = if cfg!(windows) {
        ("localflow_llm", "dll")
    } else if cfg!(target_os = "macos") {
        ("liblocalflow_llm", "dylib")
    } else {
        ("liblocalflow_llm", "so")
    };
    let dir = lib_dir();
    let path = dir.join(format!("{prefix}-{:016x}.{ext}", hash.finish()));
    if path.exists() {
        return Ok(path);
    }
    let io = |e: std::io::Error| format!("{}: {e}", dir.display());
    std::fs::create_dir_all(&dir).map_err(io)?;
    for old in std::fs::read_dir(&dir).map_err(io)?.flatten() {
        let name = old.file_name().to_string_lossy().into_owned();
        // Not another process's copy still being written (.tmp).
        if name.starts_with(prefix) && !name.ends_with(".tmp") {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let bytes = zstd::decode_all(PACKED).map_err(|e| format!("the cleanup engine is damaged: {e}"))?;
    let tmp = dir.join(format!("{prefix}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(io)?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        if !path.exists() {
            return Err(io(e)); // (another process wrote it first: use theirs)
        }
    }
    Ok(path)
}

fn lib() -> Result<&'static Lib, String> {
    static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let path = unpack()?;
        let failed = |e: libloading::Error| format!("couldn't load the cleanup engine ({}): {e}", path.display());
        // Loaded once and kept for the life of the process.
        let lib: &'static libloading::Library =
            Box::leak(Box::new(unsafe { libloading::Library::new(&path) }.map_err(failed)?));
        unsafe {
            Ok(Lib {
                load: *lib.get::<Load>(b"lf_load\0").map_err(failed)?,
                complete: *lib.get::<Complete>(b"lf_complete\0").map_err(failed)?,
                free_string: *lib.get::<FreeString>(b"lf_free_string\0").map_err(failed)?,
                free: *lib.get::<Free>(b"lf_free\0").map_err(failed)?,
            })
        }
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A message the library allocated, freed.
fn take(lib: &Lib, s: *mut c_char) -> String {
    if s.is_null() {
        return "unknown error".into();
    }
    let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    unsafe { (lib.free_string)(s) };
    text
}

/// A loaded model with its context: one completion at a time.
pub struct Model {
    lib: &'static Lib,
    ptr: *mut c_void,
}

// The model is only used through &mut, from one thread at a time.
unsafe impl Send for Model {}

impl Model {
    /// Load a GGUF file. `gpu`: every layer on the GPU when there is one.
    /// `n_ctx`: room for prompt and answer together, in tokens.
    pub fn load(path: &Path, gpu: bool, n_ctx: u32) -> Result<Model, String> {
        let lib = lib()?;
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut error = std::ptr::null_mut();
        let ptr = unsafe { (lib.load)(c_path.as_ptr(), gpu as c_int, n_ctx as c_int, &mut error) };
        if ptr.is_null() {
            return Err(take(lib, error));
        }
        Ok(Model { lib, ptr })
    }

    /// The greedy continuation of `prompt` (special tokens are parsed), up to
    /// the end of the turn; true when it stopped at `max_tokens` instead.
    pub fn complete(&mut self, prompt: &str, max_tokens: usize) -> Result<(String, bool), String> {
        let c_prompt = CString::new(prompt).map_err(|e| e.to_string())?;
        let (mut cut_off, mut error) = (0, std::ptr::null_mut());
        let max = max_tokens.min(c_int::MAX as usize) as c_int;
        let out = unsafe { (self.lib.complete)(self.ptr, c_prompt.as_ptr(), max, &mut cut_off, &mut error) };
        if out.is_null() {
            return Err(take(self.lib, error));
        }
        Ok((take(self.lib, out), cut_off != 0))
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { (self.lib.free)(self.ptr) };
    }
}
