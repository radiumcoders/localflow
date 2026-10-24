//! Speech models: whisper.cpp's ggml files from Hugging Face, kept in a
//! per-user folder, downloaded (checked by size and SHA-256) when you pick one.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::{Error, Result, err};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// What LOCALFLOW_SPEECH_MODEL names: "small.en".
    pub id: &'static str,
    pub name: &'static str,
    /// The file name, in the repository and on disk.
    pub file: &'static str,
    /// Hugging Face repository it comes from.
    pub repo: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    pub sha256: &'static str,
    /// English only (".en"): faster and more accurate for English, no other languages.
    pub english_only: bool,
    pub note: &'static str,
}

impl Model {
    pub fn url(&self) -> String {
        format!("https://huggingface.co/{}/resolve/main/{}", self.repo, self.file)
    }

    pub fn size_mb(&self) -> u64 {
        self.size.div_ceil(1_000_000)
    }

    /// Where it lives (or would) on this machine.
    pub fn path(&self) -> PathBuf {
        models_dir().join(self.file)
    }

    /// Downloaded and complete.
    pub fn installed(&self) -> bool {
        fs::metadata(self.path()).is_ok_and(|m| m.len() == self.size)
    }
}

const WHISPER: &str = "ggerganov/whisper.cpp";

/// The speech models to choose from, smallest first. Quantized (q5/q8) files:
/// about a third of the size of the originals, at nearly the same accuracy.
pub const MODELS: &[Model] = &[
    Model {
        id: "tiny.en",
        name: "Tiny (English)",
        file: "ggml-tiny.en-q8_0.bin",
        repo: WHISPER,
        size: 43_550_795,
        sha256: "5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94",
        english_only: true,
        note: "Fastest and smallest; misses more words",
    },
    Model {
        id: "base.en",
        name: "Base (English)",
        file: "ggml-base.en-q8_0.bin",
        repo: WHISPER,
        size: 81_781_811,
        sha256: "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e",
        english_only: true,
        note: "Fast on any CPU, little memory; fine for clear speech",
    },
    Model {
        id: "small.en",
        name: "Small (English)",
        file: "ggml-small.en-q5_1.bin",
        repo: WHISPER,
        size: 190_098_681,
        sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        english_only: true,
        note: "Accurate, still quick on a CPU",
    },
    Model {
        id: "large-v3-turbo",
        name: "Large v3 Turbo",
        file: "ggml-large-v3-turbo-q5_0.bin",
        repo: WHISPER,
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        english_only: false,
        note: "Most accurate, every language; best with a GPU",
    },
    Model {
        id: "base",
        name: "Base (multilingual)",
        file: "ggml-base-q8_0.bin",
        repo: WHISPER,
        size: 81_768_585,
        sha256: "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
        english_only: false,
        note: "Every language, fast, little memory",
    },
    Model {
        id: "small",
        name: "Small (multilingual)",
        file: "ggml-small-q5_1.bin",
        repo: WHISPER,
        size: 190_085_487,
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        english_only: false,
        note: "Every language, accurate, quick on a CPU",
    },
];

/// Silero voice activity detection (under 1 MB): finds the parts of a
/// recording with speech, so silence is skipped instead of transcribed (where
/// whisper makes up "Thank you."), and tells a recording with no speech apart.
pub const VAD: Model = Model {
    id: "silero-vad",
    name: "Voice detection (Silero)",
    file: "ggml-silero-v6.2.0.bin",
    repo: "ggml-org/whisper-vad",
    size: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    english_only: false,
    note: "Skips silence",
};

/// The model used when none is chosen.
pub const DEFAULT: &str = "small.en";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Per-user folder for the models: ~/.local/share/localflow/models,
/// ~/Library/Application Support/localflow/models, %LOCALAPPDATA%\localflow\models.
pub fn models_dir() -> PathBuf {
    let home = || PathBuf::from(env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default());
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Local"))
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else {
        env("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    };
    base.join("localflow").join("models")
}

/// A model (or the VAD) by id.
pub fn find(id: &str) -> Option<&'static Model> {
    MODELS.iter().chain(std::iter::once(&VAD)).find(|m| m.id == id)
}

pub fn path(id: &str) -> Option<PathBuf> {
    find(id).map(Model::path)
}

pub fn installed(id: &str) -> bool {
    find(id).is_some_and(Model::installed)
}

pub fn default_model() -> &'static Model {
    find(DEFAULT).expect("the default model is in the catalog")
}

/// The model a LOCALFLOW_SPEECH_MODEL setting means: one of ours by id, else
/// the default (also for Handy's old model ids, like "handy-computer/...").
pub fn resolve(setting: Option<&str>) -> &'static Model {
    setting.map(str::trim).and_then(|id| MODELS.iter().find(|m| m.id == id)).unwrap_or_else(default_model)
}

/// Download a model (or the VAD) into models_dir(), unless it's already there.
/// `progress(done, total)` is called as bytes arrive; setting `cancel` stops
/// it (an error). A partial download resumes where it stopped.
/// A speech model brings the voice detector along (it's tiny), so silence is
/// skipped from the first dictation.
pub fn download(id: &str, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<PathBuf> {
    let model = find(id).ok_or_else(|| Error(format!("unknown speech model {id:?}")))?;
    if model.id != VAD.id && !VAD.installed() {
        download(VAD.id, &mut |_, _| {}, cancel)?;
    }
    let dest = model.path();
    download_file(model.name, &model.url(), model.size, model.sha256, &dest, progress, cancel)?;
    Ok(dest)
}

/// Download `url` to `dest`, checked by size and SHA-256 (any file, the
/// cleanup model too). Bytes come in to `dest`.part first, and a partial one
/// resumes where it stopped. `name` is for messages.
pub fn download_file(name: &str, url: &str, size: u64, sha256: &str, dest: &Path,
                     progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
    if fs::metadata(dest).is_ok_and(|m| m.len() == size) {
        progress(size, size);
        return Ok(());
    }
    let dir = dest.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    fetch(&File { name, size, sha256 }, url, &part, progress, cancel)?;
    fs::rename(&part, dest).map_err(|e| Error(format!("{}: {e}", dest.display())))
}

/// What fetch checks a download against.
struct File<'a> {
    name: &'a str,
    size: u64,
    sha256: &'a str,
}

fn fetch(model: &File, url: &str, part: &Path, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
    let io = |e: std::io::Error| Error(format!("{}: {e}", part.display()));
    // What an earlier, interrupted download got: hash it, then ask for the rest.
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut f) = fs::File::open(part) {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf).map_err(io)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }
    if have > model.size {
        fs::remove_file(part).map_err(io)?;
        return fetch(model, url, part, progress, cancel);
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent("localflow")
        .build();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| Error(format!("couldn't download {}: {e}", model.name)))?;
    let mut out = if have > 0 && resp.status() == 206 {
        OpenOptions::new().append(true).open(part).map_err(io)?
    } else {
        // The server sent the whole file: start over.
        hasher = Sha256::new();
        have = 0;
        fs::File::create(part).map_err(io)?
    };
    let mut body = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    progress(have, model.size);
    while have < model.size {
        if cancel.load(Ordering::Relaxed) {
            return err("download cancelled");
        }
        let n = body.read(&mut buf).map_err(|e| Error(format!("couldn't download {}: {e}", model.name)))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io)?;
        hasher.update(&buf[..n]);
        have += n as u64;
        progress(have, model.size);
    }
    out.flush().map_err(io)?;
    drop(out);
    if have != model.size {
        return err(format!("{} download stopped early ({have} of {} bytes); try again", model.name, model.size));
    }
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != model.sha256 {
        let _ = fs::remove_file(part);
        return err(format!("{} download is damaged (checksum mismatch); try again", model.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog() {
        let mut ids: Vec<_> = MODELS.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len(), "ids are unique");
        for m in MODELS.iter().chain([&VAD]) {
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(m.file.starts_with("ggml-") && m.file.ends_with(".bin"));
            assert!(m.url().starts_with("https://huggingface.co/") && m.url().ends_with(m.file));
            assert_eq!(m.english_only, m.id.ends_with(".en"));
        }
        assert!(MODELS.windows(2).take(3).all(|w| w[0].size < w[1].size), "English ones smallest first");
        assert!(find(DEFAULT).is_some());
        assert_eq!(find("silero-vad"), Some(&VAD));
    }

    #[test]
    fn resolve_settings() {
        assert_eq!(resolve(None).id, DEFAULT);
        assert_eq!(resolve(Some("base.en")).id, "base.en");
        assert_eq!(resolve(Some(" tiny.en ")).id, "tiny.en");
        // Handy's ids, from before localflow had its own engine
        assert_eq!(resolve(Some("handy-computer/parakeet-unified-en-0.6b-gguf/x.gguf")).id, DEFAULT);
        assert_eq!(resolve(Some("parakeet-tdt-0.6b-v3")).id, DEFAULT);
        assert_eq!(resolve(Some("silero-vad")).id, DEFAULT, "the VAD isn't a speech model");
    }

    #[test]
    fn models_dir_is_per_user() {
        let dir = models_dir();
        assert!(dir.ends_with("localflow/models"), "{}", dir.display());
    }
}
