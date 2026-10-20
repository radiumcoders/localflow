//! Paste into the focused window: put the text on the clipboard, press the
//! paste keys, then restore the clipboard a moment later. Pasting instead of
//! typing keeps a list's newlines from pressing Enter. When the keys can't be
//! sent, the text stays on the clipboard and these return false.

use localflow_core::Part;

/// Paste `text` where you're typing.
pub fn paste_text(text: &str) -> bool {
    paste_parts(&[Part::Text(text.to_string())])
}

/// Paste parts in order: text, or a file (an absolute path) pasted as the
/// file itself, an attachment. The clipboard is restored once, at the end.
pub fn paste_parts(parts: &[Part]) -> bool {
    #[cfg(windows)]
    return crate::windows::paste(parts);
    #[cfg(target_os = "macos")]
    return crate::macos::paste(parts);
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::paste(parts)
}

/// A key pressed on its own (a spoken command, or sending a prompt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Escape,
}

/// Press `key` in the focused window. False when it can't be sent.
pub fn press(key: Key) -> bool {
    #[cfg(windows)]
    return crate::windows::press_key(key);
    #[cfg(target_os = "macos")]
    return crate::macos::press_key(key);
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::press_key(key)
}

/// Text you may have selected or copied, most likely first (the learn key's
/// input). Linux reads the selection directly; macOS and Windows copy it,
/// then put the clipboard back.
pub fn selected_text() -> Vec<String> {
    #[cfg(windows)]
    return crate::windows::selected_text();
    #[cfg(target_os = "macos")]
    return crate::macos::selected_text();
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::selected_text()
}

/// The clipboard's text (None if there's none or it can't be read).
pub fn clipboard_get() -> Option<String> {
    #[cfg(windows)]
    return crate::windows::clipboard_get();
    #[cfg(target_os = "macos")]
    return crate::macos::clipboard_get();
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::clipboard_get()
}

/// Put text on the clipboard.
pub fn clipboard_set(text: &str) -> bool {
    #[cfg(windows)]
    return crate::windows::clipboard_set(text);
    #[cfg(target_os = "macos")]
    return crate::macos::clipboard_set(text);
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::clipboard_set(text)
}

/// Put a file on the clipboard, as the file manager's Copy does; true if the
/// clipboard then holds a file (the self-test's attachment check).
pub fn clipboard_set_file(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    return crate::windows::clipboard_set_file(path);
    #[cfg(target_os = "macos")]
    return crate::macos::clipboard_set_file(path);
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::clipboard_set_file(path)
}

/// VS Code-family editors: their chat panels (Claude Code's in Cursor) ignore
/// a synthetic Ctrl+V, so they get Shift+Insert.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn is_editor(class: &str) -> bool {
    ["cursor", "code", "code-oss", "codium", "vscodium", "windsurf"].contains(&class.to_lowercase().as_str())
}

/// wtype's arguments for the paste keys on Wayland: Shift+Insert in terminals
/// (a Hyprland "terminal" tag, as Omarchy's universal paste checks) and
/// editors, Ctrl+V elsewhere.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub(crate) fn wayland_keys(class: &str, tags: &[String]) -> [&'static str; 6] {
    if tags.iter().any(|t| t == "terminal" || t == "terminal*") || is_editor(class) {
        ["-M", "shift", "-k", "Insert", "-m", "shift"]
    } else {
        ["-M", "ctrl", "-k", "v", "-m", "ctrl"]
    }
}

/// xdotool's paste key on X11: Ctrl+Shift+V in terminals.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub(crate) fn x11_key(class: &str) -> &'static str {
    let terminal = regex::Regex::new("term|kitty|alacritty|ghostty|konsole|tilix|wezterm|foot|terminator").unwrap();
    let class = class.to_lowercase();
    if terminal.is_match(&class) {
        "ctrl+shift+v"
    } else if is_editor(&class) {
        "shift+Insert"
    } else {
        "ctrl+v"
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod linux {
    //! wl-clipboard + wtype on Wayland, xclip (or xsel) + xdotool on X11.

    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use localflow_core::Part;

    use super::{Key, wayland_keys, x11_key};
    use crate::project::file_uri;
    use crate::target::hyprland_window;
    use crate::{command, output, run, which};

    fn wayland() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some() && which("wl-copy").is_some()
    }

    /// Run `cmd` with `input` on stdin. The clipboard tools stay behind to
    /// serve the clipboard, so their output isn't waited for.
    fn feed(cmd: &mut Command, input: &[u8]) -> bool {
        let Ok(mut child) = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            return false;
        };
        let wrote = child.stdin.take().is_some_and(|mut s| s.write_all(input).is_ok());
        child.wait().is_ok_and(|s| s.success()) && wrote
    }

    fn quiet(cmd: &mut Command) -> bool {
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn bytes(cmd: &mut Command) -> Vec<u8> {
        run(cmd, Duration::from_secs(2)).filter(|o| o.status.success()).map(|o| o.stdout).unwrap_or_default()
    }

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    /// Put the old clipboard back in a second, from a process of its own, so
    /// it happens even if this one exits first.
    fn restore_later(old: Vec<u8>, copy: &[&str]) {
        use std::os::unix::fs::OpenOptionsExt;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let file = std::env::temp_dir().join(format!("localflow-clipboard-{}-{nanos}", std::process::id()));
        let saved = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file)
            .and_then(|mut f| f.write_all(&old));
        if old.is_empty() || saved.is_err() {
            let _ = std::fs::remove_file(&file);
            return;
        }
        let mut cmd = command("sh");
        cmd.arg("-c").arg(r#"sleep 1; f=$1; shift; "$@" <"$f"; rm -f "$f""#).arg("sh").arg(&file).args(copy);
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    }

    pub fn paste(parts: &[Part]) -> bool {
        if wayland() { paste_wayland(parts) } else if std::env::var_os("DISPLAY").is_some() { paste_x11(parts) } else { false }
    }

    fn paste_wayland(parts: &[Part]) -> bool {
        let types = output(command("wl-paste").arg("--list-types"), Duration::from_secs(2));
        let old_type = types.lines().next().unwrap_or_default().to_string();
        let old = if old_type.is_empty() { vec![] } else { bytes(command("wl-paste").args(["--no-newline", "--type", &old_type])) };
        let (mut class, mut tags) = (String::new(), vec![]);
        if which("hyprctl").is_some() {
            let w = hyprland_window(&output(command("hyprctl").args(["activewindow", "-j"]), Duration::from_secs(2)));
            (class, tags) = (w.app, w.tags);
        }
        let keys = wayland_keys(&class, &tags);
        for (i, part) in parts.iter().enumerate() {
            match part {
                Part::File(path) => feed(command("wl-copy").args(["--type", "text/uri-list"]), format!("{}\r\n", file_uri(path.as_ref())).as_bytes()),
                Part::Text(text) => feed(&mut command("wl-copy"), text.as_bytes()),
            };
            sleep(50);
            if !quiet(command("wtype").args(keys)) {
                return false;
            }
            if i + 1 < parts.len() {
                sleep(300); // let the app read the clipboard before it changes
            }
        }
        if !old_type.is_empty() {
            restore_later(old, &["wl-copy", "--type", &old_type]);
        }
        true
    }

    fn paste_x11(parts: &[Part]) -> bool {
        let (copy, get, copy_file): (&[&str], &[&str], &[&str]) = if which("xclip").is_some() {
            (&["xclip", "-selection", "clipboard", "-in"], &["xclip", "-selection", "clipboard", "-out"],
             &["xclip", "-selection", "clipboard", "-t", "text/uri-list", "-in"])
        } else if which("xsel").is_some() {
            // xsel can't label the type: file references go in as text
            (&["xsel", "--clipboard", "--input"], &["xsel", "--clipboard", "--output"], &[])
        } else {
            return false;
        };
        let old = bytes(command(get[0]).args(&get[1..]));
        let class = output(command("xdotool").args(["getactivewindow", "getwindowclassname"]), Duration::from_secs(2));
        let key = x11_key(class.trim());
        for (i, part) in parts.iter().enumerate() {
            match part {
                Part::File(path) if !copy_file.is_empty() => {
                    feed(command(copy_file[0]).args(&copy_file[1..]), format!("{}\r\n", file_uri(path.as_ref())).as_bytes())
                }
                Part::File(text) | Part::Text(text) => feed(command(copy[0]).args(&copy[1..]), text.as_bytes()),
            };
            sleep(50);
            if !quiet(command("xdotool").args(["key", "--clearmodifiers", key])) {
                return false;
            }
            if i + 1 < parts.len() {
                sleep(300);
            }
        }
        restore_later(old, copy);
        true
    }

    pub fn press_key(key: Key) -> bool {
        let name = match key {
            Key::Enter => "Return",
            Key::Escape => "Escape",
        };
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && which("wtype").is_some() {
            quiet(command("wtype").args(["-k", name]))
        } else if std::env::var_os("DISPLAY").is_some() {
            quiet(command("xdotool").args(["key", "--clearmodifiers", name]))
        } else {
            false
        }
    }

    fn text(cmd: &mut Command) -> String {
        String::from_utf8_lossy(&bytes(cmd)).into_owned()
    }

    pub fn selected_text() -> Vec<String> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && which("wl-paste").is_some() {
            return vec![text(command("wl-paste").args(["--primary", "--no-newline"])), text(command("wl-paste").arg("--no-newline"))];
        }
        if which("xclip").is_some() {
            return ["primary", "clipboard"].map(|s| text(command("xclip").args(["-o", "-selection", s]))).to_vec();
        }
        ["-p", "-b"].map(|flag| text(command("xsel").args(["-o", flag]))).to_vec()
    }

    pub fn clipboard_get() -> Option<String> {
        if wayland() {
            return run(command("wl-paste").arg("--no-newline"), Duration::from_secs(2))
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        }
        let get: &[&str] = if which("xclip").is_some() { &["xclip", "-selection", "clipboard", "-out"] } else { &["xsel", "--clipboard", "--output"] };
        run(command(get[0]).args(&get[1..]), Duration::from_secs(2))
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    }

    pub fn clipboard_set(text: &str) -> bool {
        if wayland() {
            return feed(&mut command("wl-copy"), text.as_bytes());
        }
        if which("xclip").is_some() {
            return feed(command("xclip").args(["-selection", "clipboard", "-in"]), text.as_bytes());
        }
        feed(command("xsel").args(["--clipboard", "--input"]), text.as_bytes())
    }

    pub fn clipboard_set_file(path: &std::path::Path) -> bool {
        let uri = format!("{}\r\n", file_uri(path));
        let set = if wayland() {
            feed(command("wl-copy").args(["--type", "text/uri-list"]), uri.as_bytes())
        } else {
            feed(command("xclip").args(["-selection", "clipboard", "-t", "text/uri-list", "-in"]), uri.as_bytes())
        };
        let types = if wayland() {
            output(command("wl-paste").arg("--list-types"), Duration::from_secs(2))
        } else {
            output(command("xclip").args(["-selection", "clipboard", "-t", "TARGETS", "-out"]), Duration::from_secs(2))
        };
        set && types.lines().any(|t| t.trim() == "text/uri-list")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_keys_by_window() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(wayland_keys("brave-browser", &[]), ["-M", "ctrl", "-k", "v", "-m", "ctrl"]);
        assert_eq!(wayland_keys("com.mitchellh.ghostty", &tags(&["terminal*"]))[3], "Insert");
        assert_eq!(wayland_keys("whatever", &tags(&["terminal"]))[3], "Insert");
        assert_eq!(wayland_keys("Cursor", &[])[3], "Insert");
        assert_eq!(wayland_keys("cursor-notes", &tags(&["terminals-not"]))[3], "v");
        assert_eq!(x11_key("Alacritty"), "ctrl+shift+v");
        assert_eq!(x11_key("Code"), "shift+Insert");
        assert_eq!(x11_key("firefox"), "ctrl+v");
    }
}
