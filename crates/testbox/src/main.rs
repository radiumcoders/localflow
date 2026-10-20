//! A text box for the end-to-end tests (`cargo xtask e2e`): it takes the
//! focus, holds `INITIAL` (selected) if given, and when the file `FLAG`
//! appears writes what it holds to `OUT` and exits.
//!
//!     localflow-testbox OUT FLAG [INITIAL]
//!
//! It logs what happens to stderr, for when a dictation doesn't land.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::*;

struct Box_ {
    text: Entity<TextareaState>,
}

impl Render for Box_ {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_2().child(Textarea::new(&self.text).size_full())
    }
}

fn say(start: Instant, msg: impl std::fmt::Display) {
    eprintln!("  [text box {:5.1}s] {msg}", start.elapsed().as_secs_f32());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(flag)) = (args.first().map(PathBuf::from), args.get(1).map(PathBuf::from)) else {
        eprintln!("usage: localflow-testbox OUT FLAG [INITIAL]");
        std::process::exit(2);
    };
    let initial = args.get(2).cloned().unwrap_or_default();
    let start = Instant::now();
    application().with_assets(gpui_kit::assets::AllAssets).run(move |cx: &mut App| {
        gpui_kit::init(cx);
        let bounds = Bounds::new(point(px(100.), px(100.)), size(px(500.), px(200.)));
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some("e2e target".into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("localflow-testbox".into()),
            focus: true,
            ..Default::default()
        };
        let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
            let text = cx.new(|cx| TextareaState::new(window, cx).default_value(initial.clone()));
            cx.new(|_| Box_ { text })
        })
        .expect("couldn't open the text box");
        let text = view.read(cx).text.clone();
        let grab = move |cx: &mut App| {
            let _ = window.update(cx, |_, window, cx| {
                cx.activate(true);
                window.activate_window();
                take_foreground(window);
                text.update(cx, |t, cx| {
                    t.focus(window, cx);
                    if !t.value().is_empty() {
                        t.select_all(window, cx);
                    }
                });
            });
            say(start, "focused");
        };
        grab(cx);
        cx.spawn(async move |cx| {
            let mut ticks = 0u32;
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                ticks += 1;
                if ticks == 7 {
                    cx.update(|cx| grab(cx)); // again, once the window is up everywhere
                }
                if flag.exists() || start.elapsed() > Duration::from_secs(170) {
                    let got = cx.update(|cx| view.read(cx).text.read(cx).value().to_string());
                    say(start, format_args!("done, got {got:?}"));
                    let _ = std::fs::write(&out, got.trim());
                    cx.update(|cx| cx.quit());
                    break;
                }
            }
        })
        .detach();
    });
}

/// Windows only lets the foreground app hand over the foreground; borrow its
/// input queue for a moment (a lone Alt press would open the window menu).
fn take_foreground(window: &mut Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            AttachThreadInput, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        };
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(h) = handle.as_raw()
        {
            let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            unsafe {
                let fg = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
                let me = GetCurrentThreadId();
                AttachThreadInput(me, fg, 1);
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
                SetFocus(hwnd);
                AttachThreadInput(me, fg, 0);
            }
        }
    }
    // X11 without a window manager (Xvfb in CI): nobody hands out the input
    // focus, so take it.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some() {
        let _ = std::process::Command::new("xdotool")
            .args(["search", "--name", "^e2e target$", "windowfocus"])
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = window;
}
