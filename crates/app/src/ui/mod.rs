//! The GPUI side: the overlay pill and the settings window, on the main
//! thread. Everything else (recording, transcription, cleanup, paste) runs on
//! worker threads and talks to it through `Ui`.

pub mod jobs;
pub mod look;
pub mod onboarding;
pub mod overlay;
pub mod settings;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use gpui_kit::component::theme::Theme;
use gpui_kit::*;

use overlay::{Overlay, Phase};

/// What a worker thread asks the UI to do.
#[derive(Debug, Clone, PartialEq)]
pub enum UiMsg {
    Recording { raw: bool },
    Working(String),
    /// A message, shown for this many milliseconds.
    Message(String, u64),
    Hide,
    Quit,
}

/// A handle to the UI for worker threads. Cheap to clone.
#[derive(Clone)]
pub struct Ui {
    tx: async_channel::Sender<UiMsg>,
    level: Arc<AtomicU32>,
}

impl Ui {
    pub fn send(&self, msg: UiMsg) {
        let _ = self.tx.send_blocking(msg);
    }

    /// The microphone's level, 0..1; the pill draws the latest one each frame.
    pub fn level(&self, level: f32) {
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }
}

type Then = Box<dyn FnOnce(Ui, &mut App)>;

/// What to do once the app is up.
pub struct Start {
    /// Open the settings window right away.
    pub settings: bool,
    /// Quit when the settings window closes (it's the only thing running).
    pub quit_with_settings: bool,
    /// Open the onboarding rather than the settings, even when set up.
    pub onboarding: bool,
    /// Runs on the main thread once the UI is ready, with a handle for workers.
    pub then: Then,
}

/// The UI's handle and its end of the channel: made before the UI runs, so
/// recording can start (and send levels) while the window is still coming up.
pub fn channel() -> (Ui, async_channel::Receiver<UiMsg>) {
    let (tx, rx) = async_channel::unbounded();
    (Ui { tx, level: Arc::default() }, rx)
}

/// Run the UI until something sends `UiMsg::Quit` (or, with
/// `quit_with_settings`, the settings window closes).
pub fn run(ui: Ui, rx: async_channel::Receiver<UiMsg>, start: Start) {
    application().with_assets(gpui_kit::assets::AllAssets).with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        gpui_kit::init(cx);
        Theme::sync_system_appearance(None, cx);
        look::init(cx);
        let theme = Theme::global_mut(cx);
        theme.font_family = look::SANS.into();
        theme.mono_font_family = look::MONO.into();
        jobs::Jobs::init(cx);
        crate::platform::background_app();
        Overlay::init(ui.level.clone(), cx);
        settings::init(start.quit_with_settings, start.onboarding, cx);
        cx.spawn(async move |cx| {
            while let Ok(msg) = rx.recv().await {
                let quit = msg == UiMsg::Quit;
                cx.update(|cx| handle(msg, cx));
                if quit {
                    break;
                }
            }
        })
        .detach();
        if start.settings {
            settings::open(cx);
        }
        (start.then)(ui, cx);
    });
}

fn handle(msg: UiMsg, cx: &mut App) {
    match msg {
        UiMsg::Recording { raw } => Overlay::show(Phase::Recording { raw, since: Instant::now() }, None, cx),
        UiMsg::Working(text) => Overlay::show(Phase::Working(text.into()), None, cx),
        UiMsg::Message(text, ms) => Overlay::show(Phase::Message(text.into()), Some(ms), cx),
        UiMsg::Hide => Overlay::hide(cx),
        UiMsg::Quit => cx.quit(),
    }
}
