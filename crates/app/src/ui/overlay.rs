//! The pill at the bottom of the screen while you dictate: a dot and the
//! microphone's level while recording, then what's happening ("Transcribing"),
//! then a short message when there's something to say. It never takes
//! keyboard focus, so the paste lands in the window you were typing in.
//!
//! On Wayland it's a layer-shell surface (no focus, on the monitor you're
//! on, no window rules needed); elsewhere a pop-up window that the OS is
//! told not to activate.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::*;

const BARS: usize = 22;
/// The window. The pill inside is as wide as what it shows.
const WINDOW: (f32, f32) = (360., 52.);
const PILL_H: f32 = 40.;
/// From the bottom of the screen (or of the area above the taskbar / Dock).
const MARGIN: f32 = 56.;

const BG: u32 = 0x18181b;
const EDGE: u32 = 0x2e2e33;
const FG: u32 = 0xf4f4f5;
const DIM: u32 = 0x8b8b94;
const RED: u32 = 0xef4444;
const BLUE: u32 = 0x60a5fa;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Raw: the transcript is pasted as said (a blue dot instead of red).
    Recording { raw: bool, since: Instant },
    Working(SharedString),
    Message(SharedString),
}

pub struct Pill {
    phase: Phase,
    levels: VecDeque<f32>,
    tick: u64,
    /// The microphone's latest level (f32 bits), written by the recorder.
    level: Arc<AtomicU32>,
}

impl Pill {
    fn new(phase: Phase, level: Arc<AtomicU32>) -> Pill {
        Pill { phase, levels: VecDeque::from(vec![0.0; BARS]), tick: 0, level }
    }

    fn set(&mut self, phase: Phase) {
        if matches!(phase, Phase::Recording { .. }) {
            self.levels = VecDeque::from(vec![0.0; BARS]);
        }
        self.phase = phase;
    }

    /// One animation frame (every 50 ms).
    fn step(&mut self) {
        self.tick += 1;
        if matches!(self.phase, Phase::Recording { .. }) {
            self.levels.pop_front();
            self.levels.push_back(f32::from_bits(self.level.load(Ordering::Relaxed)));
        }
    }

    fn recording(&self, raw: bool, since: Instant) -> Div {
        let speaking = self.levels.iter().rev().take(4).any(|&l| l > 0.35);
        let dot = if speaking || (self.tick / 8) % 2 == 1 { 11. } else { 8. };
        let secs = since.elapsed().as_secs();
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .size(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(dot)).rounded_full().bg(rgb(if raw { BLUE } else { RED }))),
            )
            .child(div().flex().items_center().gap(px(3.)).h(px(PILL_H)).children(self.levels.iter().map(|&l| {
                div().w(px(3.)).h(px(2. + l * (PILL_H - 16.))).rounded(px(1.5)).bg(rgb(if l > 0.2 { FG } else { DIM }))
            })))
            .child(div().w(px(30.)).text_right().text_color(rgb(DIM)).child(format!("{}:{:02}", secs / 60, secs % 60)))
    }

    fn working(&self, text: &SharedString) -> Div {
        let dots = ".".repeat(1 + (self.tick as usize / 6) % 3);
        div()
            .flex()
            .items_center()
            .child(div().text_color(rgb(FG)).child(text.clone()))
            .child(div().w(px(14.)).text_color(rgb(FG)).child(dots))
    }
}

impl Render for Pill {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.phase {
            Phase::Recording { raw, since } => self.recording(*raw, *since),
            Phase::Working(text) => self.working(text),
            Phase::Message(text) => div().text_color(rgb(FG)).child(text.clone()),
        };
        div().size_full().flex().items_end().justify_center().child(
            div()
                .h(px(PILL_H))
                .px(px(16.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(rgb(BG))
                .border_1()
                .border_color(rgb(EDGE))
                .text_size(px(13.))
                .child(content),
        )
    }
}

/// The overlay's window and what it shows. One per app.
pub struct Overlay {
    window: Option<WindowHandle<Pill>>,
    level: Arc<AtomicU32>,
    /// Bumped whenever what's shown changes, so a message's timer only hides
    /// the message it was started for.
    shown: u64,
}

impl Global for Overlay {}

impl Overlay {
    pub fn init(level: Arc<AtomicU32>, cx: &mut App) {
        cx.set_global(Overlay { window: None, level, shown: 0 });
    }

    /// Show `phase`; a message hides itself after `ms`.
    pub fn show(phase: Phase, ms: Option<u64>, cx: &mut App) {
        let overlay = cx.global_mut::<Overlay>();
        overlay.shown += 1;
        let (token, existing, level) = (overlay.shown, overlay.window, overlay.level.clone());
        let updated = existing.is_some_and(|w| {
            w.update(cx, |pill, _, cx| {
                pill.set(phase.clone());
                cx.notify();
            })
            .is_ok()
        });
        if !updated {
            match open(phase, level, cx) {
                Ok(window) => cx.global_mut::<Overlay>().window = Some(window),
                Err(e) => eprintln!("overlay: {e:#}"),
            }
        }
        if let Some(ms) = ms {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                cx.update(|cx| {
                    if cx.global::<Overlay>().shown == token {
                        Overlay::hide(cx);
                    }
                });
            })
            .detach();
        }
    }

    pub fn hide(cx: &mut App) {
        let overlay = cx.global_mut::<Overlay>();
        overlay.shown += 1;
        if let Some(window) = overlay.window.take() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    }
}

fn open(phase: Phase, level: Arc<AtomicU32>, cx: &mut App) -> anyhow::Result<WindowHandle<Pill>> {
    let size = size(px(WINDOW.0), px(WINDOW.1));
    let display = cx.primary_display();
    let area = display.as_ref().map(|d| d.visible_bounds());
    let origin = area.map_or(point(px(0.), px(0.)), |a| {
        point(a.center().x - size.width / 2., a.bottom() - size.height - px(MARGIN - (WINDOW.1 - PILL_H)))
    });
    let options = |kind| WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, size))),
        display_id: display.as_ref().map(|d| d.id()),
        app_id: Some("localflow-overlay".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Server),
        kind,
        focus: false,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let build = |level: Arc<AtomicU32>, phase: Phase| move |_: &mut Window, cx: &mut App| cx.new(|_| Pill::new(phase, level));
    let mut opened = Err(anyhow::anyhow!("no window"));
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        use layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
        let kind = WindowKind::LayerShell(LayerShellOptions {
            namespace: "localflow-overlay".into(),
            layer: Layer::Overlay,
            anchor: Anchor::BOTTOM,
            margin: Some((px(0.), px(0.), px(MARGIN - (WINDOW.1 - PILL_H)), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        });
        opened = cx.open_window(options(kind), build(level.clone(), phase.clone()));
    }
    if opened.is_err() {
        // X11, a Wayland desktop without layer-shell (GNOME), macOS, Windows.
        opened = cx.open_window(options(WindowKind::PopUp), build(level, phase));
        if let Ok(window) = &opened {
            let _ = window.update(cx, |_, window, _| crate::platform::overlay_window(window));
        }
    }
    let window = opened?;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(50)).await;
            let alive = window.update(cx, |pill, _, cx| {
                pill.step();
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
    Ok(window)
}
