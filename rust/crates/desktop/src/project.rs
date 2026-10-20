//! The project you're working in and the files you may name: the folders
//! VS Code-family editors have open, a terminal's working directory, the
//! project's files, and which of them a dictation mentions.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use localflow_core::difflib::SequenceMatcher;
use localflow_core::home;
use regex::Regex;
use serde_json::Value;

use crate::{command, run, which};

/// VS Code-family editors by window class (Linux), program (Windows) or
/// bundle id (macOS) -> their settings folder, which lists the projects they
/// have open.
pub const EDITORS: &[(&str, &str)] = &[
    ("cursor", "Cursor"),
    ("cursor.exe", "Cursor"),
    ("com.todesktop.230313mzl4w4u92", "Cursor"),
    ("code", "Code"),
    ("code.exe", "Code"),
    ("com.microsoft.vscode", "Code"),
    ("code-oss", "Code - OSS"),
    ("vscodium", "VSCodium"),
    ("codium", "VSCodium"),
    ("vscodium.exe", "VSCodium"),
    ("com.vscodium", "VSCodium"),
    ("windsurf", "Windsurf"),
    ("windsurf.exe", "Windsurf"),
    ("com.exafunction.windsurf", "Windsurf"),
];

/// Coding agents: in a terminal running several programs, the one you talk to.
const AGENTS: &[&str] =
    &["claude", "codex", "opencode", "aider", "gemini", "cursor-agent", "amp", "crush", "goose", "qwen"];
const SKIP_DIRS: &[&str] = &["node_modules", "__pycache__", "venv", "dist", "build", "target"];

/// The settings folder name of the editor with this app id, if it is one.
pub fn editor_name(app: &str) -> Option<&'static str> {
    let app = app.to_lowercase();
    EDITORS.iter().find(|(id, _)| *id == app).map(|(_, name)| *name)
}

/// Every editor settings folder name, once each, sorted.
fn editor_names() -> BTreeSet<&'static str> {
    EDITORS.iter().map(|(_, name)| *name).collect()
}

/// Where editors keep their settings: %APPDATA%, ~/Library/Application
/// Support, or $XDG_CONFIG_HOME (~/.config).
pub fn editor_base() -> PathBuf {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        var("APPDATA").unwrap_or_else(home)
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else {
        var("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config"))
    }
}

/// An editor's state database (open folders, Cursor's chat history).
pub fn editor_state_db(base: &Path, name: &str) -> PathBuf {
    base.join(name).join("User/globalStorage/state.vscdb")
}

/// Open a database other apps are writing, read-only, waiting briefly if it's busy.
pub(crate) fn open_db(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    if !path.is_file() {
        return Err(rusqlite::Error::InvalidPath(path.into()));
    }
    let db = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(Duration::from_millis(500))?;
    Ok(db)
}

/// Folders a VS Code-family editor has open or had open, most recent first,
/// from its state files.
pub fn editor_folders(base: &Path, name: &str) -> Vec<PathBuf> {
    let state = base.join(name).join("User/globalStorage");
    let mut uris: Vec<String> = vec![];
    let text = |v: &Value| v.as_str().map(String::from);
    if let Some(d) = std::fs::read_to_string(state.join("storage.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        let windows = &d["windowsState"];
        uris.extend(text(&windows["lastActiveWindow"]["folder"]));
        if let Some(opened) = windows["openedWindows"].as_array() {
            uris.extend(opened.iter().filter_map(|w| text(&w["folder"])));
        }
        if let Some(folders) = d["backupWorkspaces"]["folders"].as_array() {
            uris.extend(folders.iter().filter_map(|f| text(&f["folderUri"])));
        }
    }
    let recent = open_db(&state.join("state.vscdb")).and_then(|db| {
        db.query_row("SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'", [], |row| {
            row.get::<_, String>(0)
        })
    });
    if let Some(recent) = recent.ok().and_then(|v| serde_json::from_str::<Value>(&v).ok())
        && let Some(entries) = recent["entries"].as_array()
    {
        uris.extend(entries.iter().filter_map(|e| text(&e["folderUri"])));
    }
    let mut folders: Vec<PathBuf> = vec![];
    for path in uris.iter().filter_map(|u| file_uri_path(u)) {
        if !folders.contains(&path) && path.is_dir() {
            folders.push(path);
        }
    }
    folders
}

/// The local path of a file:// URI (percent-decoded; /c%3A/x is C:\x on Windows).
pub fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..]; // past the host, usually empty
    let mut bytes = vec![];
    let raw = path.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hex = raw.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (raw[i], hex) {
            (b'%', Some(b)) => {
                bytes.push(b);
                i += 3;
            }
            (b, _) => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8_lossy(&bytes).into_owned();
    if cfg!(windows) {
        let p = path.trim_start_matches('/');
        if p.as_bytes().get(1) == Some(&b':') {
            return Some(PathBuf::from(p.replace('/', "\\")));
        }
    }
    Some(PathBuf::from(path))
}

/// A path as a file:// URI, for pasting a file as an attachment.
pub fn file_uri(path: &Path) -> String {
    let posix = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from(if posix.starts_with('/') { "file://" } else { "file:///" });
    for b in posix.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

static TITLE_PARTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(" [-—–] ").unwrap());

/// The project in the editor window: its title is "file - folder - Cursor".
pub fn editor_project(base: &Path, name: &str, title: &str) -> Option<PathBuf> {
    let folders = editor_folders(base, name);
    let parts: Vec<&str> = TITLE_PARTS.split(title).map(|p| p.trim_matches([' ', '●'])).collect();
    folders
        .iter()
        .find(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| parts.contains(&n)))
        .or(folders.first())
        .cloned()
}

/// The project in the editor you used last, for apps that have none.
pub fn recent_project(base: &Path) -> Option<PathBuf> {
    editor_names()
        .into_iter()
        .rev() // of equally recent ones, the first (max_by keeps the last)
        .filter_map(|name| {
            let folder = editor_folders(base, name).into_iter().next()?;
            let used = std::fs::metadata(base.join(name).join("User/globalStorage/storage.json"))
                .and_then(|m| m.modified())
                .ok();
            Some((used, folder))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, folder)| folder)
}

/// {pid: (parent pid, program name)} for every process (Linux and macOS).
pub fn processes() -> HashMap<u32, (u32, String)> {
    let mut procs = HashMap::new();
    if cfg!(target_os = "linux") {
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|d| d.parse::<u32>().ok()) else { continue };
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
            let ppid = stat[close + 1..].split_whitespace().nth(1).and_then(|p| p.parse().ok());
            if let Some(ppid) = ppid {
                procs.insert(pid, (ppid, stat[open + 1..close].to_string()));
            }
        }
    } else if !cfg!(windows) {
        let out = crate::output(command("ps").args(["-axo", "pid=,ppid=,comm="]), Duration::from_secs(3));
        for line in out.lines() {
            let mut fields = line.split_whitespace();
            if let (Some(pid), Some(ppid)) = (fields.next(), fields.next()) {
                let comm = fields.collect::<Vec<_>>().join(" ");
                let name = comm.rsplit('/').next().unwrap_or_default().to_string();
                if let (Ok(pid), Ok(ppid)) = (pid.parse(), ppid.parse()) {
                    procs.insert(pid, (ppid, name));
                }
            }
        }
    }
    procs
}

/// {pid: working directory} for those of `pids` we can see.
pub fn cwds(pids: &[u32]) -> HashMap<u32, PathBuf> {
    let mut found = HashMap::new();
    if cfg!(target_os = "linux") {
        for &pid in pids {
            if let Ok(dir) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
                found.insert(pid, dir);
            }
        }
    } else if !pids.is_empty() && which("lsof").is_some() {
        let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let out = crate::output(command("lsof").args(["-a", "-d", "cwd", "-Fn", "-p", &list]), Duration::from_secs(3));
        let mut pid = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix('p') {
                pid = p.parse().ok();
            } else if let (Some(dir), Some(p)) = (line.strip_prefix('n'), pid) {
                found.insert(p, PathBuf::from(dir));
            }
        }
    }
    found
}

static TITLE_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"~?/[^\s:'"|()]*"#).unwrap());

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

/// The directory you're working in, in a terminal window: one named in its
/// title (many shells put it there), else that of a coding agent running in
/// it, else that of the newest program in it.
pub fn terminal_cwd(pid: u32, title: &str) -> Option<PathBuf> {
    for m in TITLE_PATH.find_iter(title) {
        let path = expand_home(m.as_str());
        if m.as_str().len() > 2 && path != home() && path.is_dir() {
            return Some(path);
        }
    }
    if cfg!(windows) {
        return None; // another process's directory isn't readable there
    }
    let procs = processes();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&p, (parent, _)) in &procs {
        children.entry(*parent).or_default().push(p);
    }
    let (mut under, mut todo) = (vec![], children.get(&pid).cloned().unwrap_or_default());
    while let Some(p) = todo.pop() {
        under.push(p);
        todo.extend(children.get(&p).into_iter().flatten());
    }
    // Agents first, then the newest.
    under.sort_by_key(|p| std::cmp::Reverse((AGENTS.contains(&procs[p].1.as_str()), *p)));
    under.truncate(30);
    let found = cwds(&under);
    under.iter().find_map(|p| found.get(p).cloned())
}

/// The git repository around `cwd`, or `cwd` itself (not your home folder or /).
pub fn project_root(cwd: &Path) -> Option<PathBuf> {
    if let Some(repo) = cwd.ancestors().find(|d| d.join(".git").exists()) {
        return Some(repo.to_path_buf());
    }
    (cwd != home() && cwd.parent().is_some()).then(|| cwd.to_path_buf())
}

fn git_exe() -> Option<PathBuf> {
    let git = which("git")?;
    if cfg!(target_os = "macos")
        && git == Path::new("/usr/bin/git")
        && !Path::new("/Library/Developer/CommandLineTools").exists()
        && !Path::new("/Applications/Xcode.app").exists()
    {
        return None; // the stub that offers to install the developer tools
    }
    Some(git)
}

/// The project's files and folders ("src/", with a slash), relative to root.
/// Skips what git ignores.
pub fn project_files(root: &Path) -> Vec<String> {
    const LIMIT: usize = 20000;
    let from_git = git_exe().and_then(|git| {
        let out = run(command(git).arg("-C").arg(root).args(["ls-files", "-co", "--exclude-standard", "-z"]), Duration::from_secs(3))?;
        out.status.success().then(|| {
            String::from_utf8_lossy(&out.stdout).split('\0').filter(|f| !f.is_empty()).take(LIMIT).map(String::from).collect()
        })
    });
    let files: Vec<String> = from_git.unwrap_or_else(|| walk(root, LIMIT)); // not a git repository
    let mut folders = BTreeSet::new();
    for f in &files {
        let parts: Vec<&str> = f.split('/').collect();
        for i in 1..parts.len() {
            folders.insert(parts[..i].join("/") + "/");
        }
    }
    files.into_iter().chain(folders).collect()
}

/// Files under `root`, top-down, skipping hidden and build folders.
fn walk(root: &Path, limit: usize) -> Vec<String> {
    let (mut files, mut todo) = (vec![], vec![PathBuf::new()]);
    while let Some(rel) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut dirs = vec![];
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    dirs.push(rel.join(&name));
                }
            } else {
                files.push(rel.join(&name).to_string_lossy().replace('\\', "/"));
            }
        }
        if files.len() >= limit {
            break;
        }
        todo.extend(dirs.into_iter().rev());
    }
    files
}

// Words that point at a file; without one, no file list goes to the cleanup.
static FILE_TALK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:files?|folders?|director(?:y|ies)|scripts?|modules?|components?|configs?|read ?me|dot [a-z0-9]{1,5}|attach\w*|refer\w*|mention\w*|tag)\b",
    )
    .unwrap()
});
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[a-z0-9]+").unwrap());
/// Words around a file's name ("the settings file"), not part of it.
const NAMING: &[&str] = &[
    "the", "a", "an", "file", "files", "folder", "folders", "directory", "script", "module", "component", "config", "to",
    "at", "and", "in", "of", "on", "my", "our", "this", "that", "look", "refer", "check",
];

fn compact(t: &str) -> String {
    t.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

/// difflib's SequenceMatcher(None, a, b).ratio() over characters.
fn ratio(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let same: usize = SequenceMatcher::new(&a, &b).matching_blocks().iter().map(|m| m.size).sum();
    2.0 * same as f64 / (a.len() + b.len()) as f64
}

/// difflib.get_close_matches(word, possibilities, n, cutoff): the best
/// matches scoring at least `cutoff`, best first.
fn close_matches<'a>(word: &str, possibilities: impl Iterator<Item = &'a String>, n: usize, cutoff: f64) -> Vec<&'a String> {
    let mut scored: Vec<(f64, &String)> =
        possibilities.map(|x| (ratio(x, word), x)).filter(|(score, _)| *score >= cutoff).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(a.1)));
    scored.into_iter().take(n).map(|(_, x)| x).collect()
}

/// The project paths the dictation most likely names, best first; empty
/// when it doesn't sound like it mentions a file.
pub fn file_candidates<'a>(text: &str, files: &'a [String], limit: usize) -> Vec<String> {
    if files.is_empty() || !FILE_TALK.is_match(text) {
        return vec![];
    }
    let mut index: HashMap<String, HashSet<&'a String>> = HashMap::new();
    for path in files {
        let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
        let stem = if name.starts_with('.') { name } else { name.split('.').next().unwrap_or_default() };
        for key in HashSet::from([compact(name), compact(stem), compact(path)]) {
            if key.chars().count() >= 3 {
                index.entry(key).or_default().insert(path);
            }
        }
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = WORDS.find_iter(&lower).map(|m| m.as_str()).collect();
    let mut grams = HashSet::new();
    for n in 1..5 {
        // "settings", "dictate dot py", "test local flow", "read me"
        for ws in words.windows(n) {
            let (first, last) = (ws[0], ws[n - 1]);
            if !NAMING.contains(&first) && !NAMING.contains(&last) && last != "dot" {
                let gram: String = ws.iter().filter(|w| **w != "dot").copied().collect();
                if gram.chars().count() >= 3 {
                    grams.insert(gram);
                }
            }
        }
    }
    let mut score: HashMap<&'a String, f64> = HashMap::new();
    let mut raise = |path: &'a String, s: f64| {
        let e = score.entry(path).or_insert(s);
        *e = e.max(s);
    };
    for g in &grams {
        for path in index.get(g).into_iter().flatten() {
            raise(path, 2.0 * g.chars().count() as f64);
        }
    }
    if grams.len() * index.len() <= 400_000 {
        // near misses ("e to e" for e2e, "settings" for setting)
        for g in grams.iter().filter(|g| g.chars().count() >= 5) {
            for key in close_matches(g, index.keys(), 3, 0.8) {
                let r = ratio(g, key);
                for path in &index[key] {
                    raise(path, r * g.chars().count() as f64);
                }
            }
        }
    }
    let mut best: Vec<(&String, f64)> = score.into_iter().collect();
    best.sort_by(|(p, s), (q, t)| {
        t.total_cmp(s)
            .then_with(|| p.matches('/').count().cmp(&q.matches('/').count()))
            .then_with(|| p.len().cmp(&q.len()))
            .then_with(|| p.cmp(q))
    });
    best.into_iter().take(limit).map(|(p, _)| p.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<String> {
        ["README.md", "install.py", "src/settings.py", "src/components/Button.tsx", "tests/test_settings.py", "src/",
         "src/components/", "tests/"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn candidates() {
        let (f, c) = (files(), |t: &str, f: &[String]| file_candidates(t, f, 12));
        assert_eq!(c("refer to the install file", &f)[0], "install.py");
        assert_eq!(c("update the read me", &f)[0], "README.md");
        assert_eq!(c("check test settings dot py", &f)[0], "tests/test_settings.py");
        assert!(c("look at the button component", &f).contains(&"src/components/Button.tsx".to_string()));
        assert!(c("we need to install the package", &f).is_empty()); // no word for a file
        assert!(c("look at the file", &[]).is_empty());
        // Near misses, as Python's difflib finds them.
        let f = ["src/setting.py", "src/", "tests/e2e.py", "tests/"].map(String::from);
        assert_eq!(c("open the settings file", &f), ["src/setting.py"]);
        let f = ["dictate", "localflow", "tests/test_localflow.py", "tests/", "pkg/main.py", "pkg/"].map(String::from);
        assert_eq!(c("look at the dictation script and the local flow test", &f), ["localflow", "tests/test_localflow.py"]);
    }

    #[test]
    fn ratios_match_difflib() {
        assert_eq!(ratio("abcd", "bcde"), 0.75);
        assert_eq!(close_matches("appel", [&"ape".to_string(), &"apple".to_string(), &"peach".to_string()].into_iter(), 3, 0.6),
                   vec!["apple", "ape"]);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("localflow-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn editor_projects() {
        let base = temp("editor");
        let (a, b) = (base.join("alpha"), base.join("beta"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let state = base.join("Cursor/User/globalStorage");
        std::fs::create_dir_all(&state).unwrap();
        let storage = serde_json::json!({"windowsState": {
            "lastActiveWindow": {"folder": file_uri(&a)}, "openedWindows": [{"folder": file_uri(&b)}]}});
        std::fs::write(state.join("storage.json"), storage.to_string()).unwrap();
        // And one more from the recently-opened list in the database.
        let c = base.join("gamma dir");
        std::fs::create_dir_all(&c).unwrap();
        let db = rusqlite::Connection::open(state.join("state.vscdb")).unwrap();
        db.execute("CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)", []).unwrap();
        let recent = serde_json::json!({"entries": [{"folderUri": file_uri(&b)}, {"folderUri": file_uri(&c)}]});
        db.execute("INSERT INTO ItemTable VALUES ('history.recentlyOpenedPathsList', ?1)", [recent.to_string()]).unwrap();
        drop(db);
        assert_eq!(editor_folders(&base, "Cursor"), vec![a.clone(), b.clone(), c]);
        assert_eq!(editor_project(&base, "Cursor", "● main.py - beta - Cursor"), Some(b));
        assert_eq!(editor_project(&base, "Cursor", "Welcome - Cursor"), Some(a.clone())); // the last one used
        assert_eq!(recent_project(&base), Some(a));
        assert_eq!(editor_project(&base, "Code", "x - Code"), None);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn files_without_git() {
        let root = temp("files");
        for f in ["a.py", "src/b.py", "src/ui/c.tsx", "node_modules/x/y.js", ".hidden/z"] {
            std::fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            std::fs::write(root.join(f), "").unwrap();
        }
        let mut found = project_files(&root);
        found.sort();
        assert_eq!(found, ["a.py", "src/", "src/b.py", "src/ui/", "src/ui/c.tsx"]);
        assert_eq!(project_root(&root.join("src")), Some(root.join("src"))); // no git: the folder itself
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn files_with_git() {
        if git_exe().is_none() {
            return;
        }
        let root = temp("git");
        for f in ["a.py", "src/b.py", "ignored/c.py"] {
            std::fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            std::fs::write(root.join(f), "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        let ok = command("git").arg("-C").arg(&root).args(["init", "-q"]).status().is_ok_and(|s| s.success());
        assert!(ok);
        let mut found = project_files(&root);
        found.sort();
        assert_eq!(found, [".gitignore", "a.py", "src/", "src/b.py"]);
        assert_eq!(project_root(&root.join("src")), Some(root.clone()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn uris() {
        if !cfg!(windows) {
            let p = Path::new("/tmp/a b/ü.py");
            assert_eq!(file_uri(p), "file:///tmp/a%20b/%C3%BC.py");
            assert_eq!(file_uri_path(&file_uri(p)), Some(p.to_path_buf()));
            assert_eq!(file_uri_path("file://host/x"), Some(PathBuf::from("/x")));
        }
        assert_eq!(file_uri_path("https://x/y"), None);
    }
}
