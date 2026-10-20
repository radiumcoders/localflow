//! The window a dictation starts in, the project open there, and how that app
//! takes a reference to one of the project's files.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
#[cfg(not(any(windows, target_os = "macos")))]
use std::time::Duration;

use localflow_core::{Part, file_refs};
use regex::Regex;
use serde_json::Value;

use crate::project::{editor_base, editor_name, editor_project, project_files, project_root, recent_project, terminal_cwd};
#[cfg(not(any(windows, target_os = "macos")))]
use crate::{command, output, which};

/// The focused window. `app` is the window class (Linux), program file
/// (Windows) or bundle id (macOS); `tags` are Hyprland's ("terminal*").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    pub app: String,
    pub title: String,
    pub pid: Option<u32>,
    pub tags: Vec<String>,
}

/// The window you're typing in (empty where the desktop doesn't say: Wayland
/// outside Hyprland).
pub fn focused_window() -> Window {
    #[cfg(windows)]
    return crate::windows::focused_window();
    #[cfg(target_os = "macos")]
    return crate::macos::focused_window();
    #[cfg(not(any(windows, target_os = "macos")))]
    linux_focused_window()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn linux_focused_window() -> Window {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return hyprland_window(&output(command("hyprctl").args(["activewindow", "-j"]), Duration::from_secs(2)));
    }
    if std::env::var_os("DISPLAY").is_some() && which("xdotool").is_some() {
        let get = |what| output(command("xdotool").args(["getactivewindow", what]), Duration::from_secs(2)).trim().to_string();
        return Window {
            app: get("getwindowclassname"),
            title: get("getwindowname"),
            pid: get("getwindowpid").parse().ok(),
            tags: vec![],
        };
    }
    Window::default()
}

/// A window from `hyprctl activewindow -j`.
pub fn hyprland_window(json: &str) -> Window {
    let w: Value = serde_json::from_str(json).unwrap_or_default();
    let text = |k: &str| w[k].as_str().unwrap_or_default().to_string();
    Window {
        app: text("class"),
        title: text("title"),
        pid: w["pid"].as_u64().map(|p| p as u32),
        tags: w["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect(),
    }
}

/// Something that identifies the focused window, to check the overlay
/// doesn't take focus (None where it can't be told).
pub fn foreground() -> Option<String> {
    #[cfg(windows)]
    return crate::windows::foreground();
    #[cfg(target_os = "macos")]
    return crate::macos::frontmost_pid().map(|p| p.to_string());
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let out = output(command("hyprctl").args(["activewindow", "-j"]), Duration::from_secs(2));
        let w: Value = serde_json::from_str(&out).ok()?;
        w.get("address").map(|a| format!("{} {}", w["class"].as_str().unwrap_or_default(), a.as_str().unwrap_or_default()))
    }
}

static TERMINALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)term|kitty|alacritty|ghostty|konsole|tilix|wezterm|foot|iterm|warp|tabby|^cmd\.exe$|powershell|pwsh|mintty")
        .unwrap()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Cursor, VS Code and the like.
    Editor,
    Terminal,
    /// A browser, Slack, mail...
    Other,
}

/// The window a dictation started in, and the project open there.
#[derive(Debug, Clone)]
pub struct Target {
    pub kind: Kind,
    pub app: String,
    pub pid: Option<u32>,
    /// A terminal's working directory.
    pub cwd: Option<PathBuf>,
    pub project: Option<PathBuf>,
    /// The project's files and folders ("src/"), relative to it.
    pub files: Vec<String>,
}

/// How an app takes a reference to a file: as text ("@src/app.py"), or the
/// file itself, pasted as an attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Text(String),
    File(PathBuf),
}

impl Target {
    /// Work out the app's kind and project (reads the editor's state, the
    /// terminal's processes and the project's files: run it off the UI thread).
    pub fn new(window: &Window) -> Target {
        let app = window.app.to_lowercase();
        let kind = if editor_name(&app).is_some() {
            Kind::Editor
        } else if TERMINALS.is_match(&app) || window.tags.iter().any(|t| t.starts_with("terminal")) {
            Kind::Terminal
        } else {
            Kind::Other
        };
        let base = editor_base();
        let (mut cwd, mut project) = (None, None);
        match (kind, window.pid) {
            (Kind::Editor, _) => project = editor_project(&base, editor_name(&app).unwrap_or_default(), &window.title),
            (Kind::Terminal, Some(pid)) => {
                cwd = terminal_cwd(pid, &window.title);
                project = cwd.as_deref().and_then(project_root);
            }
            _ => {}
        }
        // No project here (or none we can see): the one you last had open.
        let project = project.or_else(|| recent_project(&base));
        let files = project.as_deref().map(project_files).unwrap_or_default();
        Target { kind, app: window.app.clone(), pid: window.pid, cwd, project, files }
    }

    /// How this app takes a reference to `path` (relative to the project).
    /// Editors and terminals get text: nothing is typed, so the focus never
    /// moves. Folders can't be attached, so they're text everywhere.
    pub fn reference(&self, path: &str) -> Ref {
        let root = self.project.clone().unwrap_or_default();
        let full = root.join(path);
        let folder = path.ends_with('/');
        if self.kind == Kind::Other && !folder {
            return Ref::File(full);
        }
        let shown = match full.strip_prefix(self.cwd.as_deref().unwrap_or(&root)) {
            Ok(rel) => posix(rel),
            Err(_) => posix(&full),
        };
        let shown = shown.trim_end_matches('/');
        Ref::Text(format!("@{shown}{}", if folder { "/" } else { "" }))
    }
}

fn posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let project = self.project.as_ref().map_or("None".into(), |p| p.display().to_string());
        write!(f, "{:?} {:?}, project {project}, {} files", self.kind, self.app, self.files.len())
    }
}

/// What to paste for the cleaned text: each @path the cleanup wrote for one
/// of the `offered` files, put in the way the app takes it. One text part
/// when there's nothing to attach.
pub fn deliverable(text: &str, target: Option<&Target>, offered: &[String]) -> Vec<Part> {
    let pieces = match target {
        Some(t) if !offered.is_empty() => file_refs(text, offered)
            .into_iter()
            .map(|part| match part {
                Part::File(path) => match t.reference(&path) {
                    Ref::Text(s) => Part::Text(s),
                    Ref::File(p) => Part::File(p.to_string_lossy().into_owned()),
                },
                text => text,
            })
            .collect(),
        _ => vec![Part::Text(text.to_string())],
    };
    let mut parts: Vec<Part> = vec![];
    for piece in pieces {
        match (parts.last_mut(), piece) {
            (Some(Part::Text(prev)), Part::Text(more)) => prev.push_str(&more),
            (_, piece) => parts.push(piece),
        }
    }
    parts
}

/// Paste the cleaned text where the dictation started (see `deliverable`).
/// False when the paste keys couldn't be sent (the text is on the clipboard).
pub fn deliver(text: &str, target: Option<&Target>, offered: &[String]) -> bool {
    match deliverable(text, target, offered).as_slice() {
        [Part::Text(text)] => crate::paste::paste_text(text),
        parts => crate::paste::paste_parts(parts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: Kind, root: &Path, cwd: Option<PathBuf>) -> Target {
        Target { kind, app: "x".into(), pid: None, cwd, project: Some(root.into()), files: vec![] }
    }

    #[test]
    fn deliver_by_app() {
        let root = std::env::temp_dir().join("localflow-deliver");
        let text = "Look at @src/app.py and @tests/ now.";
        let files = ["src/app.py".to_string(), "tests/".to_string()];
        let one = |parts: Vec<Part>| match parts.as_slice() {
            [Part::Text(t)] => t.clone(),
            other => panic!("{other:?}"),
        };
        let t = target(Kind::Terminal, &root, Some(root.clone()));
        assert_eq!(one(deliverable(text, Some(&t), &files)), text);
        let t = target(Kind::Terminal, &root, Some(root.join("src")));
        assert_eq!(one(deliverable(text, Some(&t), &files)), format!("Look at @app.py and @{}/ now.", posix(&root.join("tests"))));
        let t = target(Kind::Editor, &root, None);
        assert_eq!(one(deliverable(text, Some(&t), &files)), text); // text: nothing typed
        let t = target(Kind::Other, &root, None);
        assert_eq!(deliverable(text, Some(&t), &files), vec![
            Part::Text("Look at ".into()),
            Part::File(root.join("src/app.py").to_string_lossy().into_owned()),
            Part::Text(" and @tests/ now.".into()), // a folder can't be attached
        ]);
        assert_eq!(one(deliverable("No files here.", None, &[])), "No files here.");
    }

    #[test]
    fn kinds() {
        let kind = |app: &str, tags: &[&str]| {
            let w = Window { app: app.into(), tags: tags.iter().map(|t| t.to_string()).collect(), ..Window::default() };
            // Only the kind: Target::new also reads projects, which is fine here (none match).
            Target::new(&w).kind
        };
        assert_eq!(kind("Cursor", &[]), Kind::Editor);
        assert_eq!(kind("com.mitchellh.ghostty", &[]), Kind::Terminal);
        assert_eq!(kind("WindowsTerminal.exe", &[]), Kind::Terminal);
        assert_eq!(kind("whatever", &["terminal*"]), Kind::Terminal);
        assert_eq!(kind("brave-browser", &[]), Kind::Other);
    }

    #[test]
    fn hyprland_json() {
        let w = hyprland_window(r#"{"class": "kitty", "title": "~/p", "pid": 42, "tags": ["terminal*"]}"#);
        assert_eq!(w, Window { app: "kitty".into(), title: "~/p".into(), pid: Some(42), tags: vec!["terminal*".into()] });
        assert_eq!(hyprland_window("{}"), Window::default());
    }
}
