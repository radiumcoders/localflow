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

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::look::{self, Palette, SPRING, spring};

const BARS: usize = 16;
const PILL_H: f32 = 40.;
/// Room around the pill for its shadow; the window is transparent there.
const ROOM: f32 = 16.;
/// The window: the widest pill, and its shadow.
const WINDOW: (f32, f32) = (440., PILL_H + 2. * ROOM);
/// The pill's bottom edge, from the bottom of the screen (or of the area
/// above the taskbar / Dock).
const LIFT: f32 = 44.;
const PAD: f32 = 16.;
const GAP: f32 = 10.;
const TEXT: f32 = 13.;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Raw: the transcript is pasted as said (a blue dot instead of red).
    Recording { raw: bool, since: Instant },
    Working(SharedString),
    Message(SharedString),
}

impl Phase {
    /// Changes when what's shown changes, so the new content fades in.
    fn key(&self) -> SharedString {
        match self {
            Phase::Recording { raw, .. } => format!("rec-{raw}").into(),
            Phase::Working(t) => format!("work-{t}").into(),
            Phase::Message(t) => format!("msg-{t}").into(),
        }
    }
}

/// The icon a message goes with, if any.
fn message_icon(text: &str) -> Option<&'static str> {
    match text {
        "Pasted" | "Done" => Some("check"),
        "Sent" => Some("corner-down-left"),
        t if t.starts_with("Copied") => Some("clipboard"),
        _ => None,
    }
}

pub struct Pill {
    phase: Phase,
    levels: VecDeque<f32>,
    tick: u64,
    /// The microphone's latest level (f32 bits), written by the recorder.
    level: Arc<AtomicU32>,
}

/// The pill's own colours: the site's pill, near-black on either theme.
struct Ink {
    bg: Hsla,
    fg: Hsla,
    dim: Hsla,
    quiet: Hsla,
    record: Hsla,
    raw: Hsla,
    highlight: Hsla,
    edge: Hsla,
}

impl Ink {
    fn of(window: &Window) -> Ink {
        let p = Palette::of(window);
        let white = |a: f32| Hsla { h: 0., s: 0., l: 1., a };
        let dark = p.bg.l < 0.5;
        Ink {
            bg: if dark { rgb(0x262626).into() } else { rgb(0x0a0a0a).into() },
            fg: white(1.),
            dim: white(0.55),
            quiet: white(0.28),
            record: Palette::DARK.record,
            raw: Palette::DARK.primary,
            highlight: white(if dark { 0.08 } else { 0.12 }),
            edge: Hsla { h: 0., s: 0., l: 0., a: if dark { 0.6 } else { 0.25 } },
        }
    }
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

    /// How wide the pill is for what it shows, so it can spring to it.
    fn width(&self, window: &Window) -> f32 {
        let text = |t: &str| look::text_width(t, TEXT, FontWeight::MEDIUM, window);
        let inner = match &self.phase {
            Phase::Recording { .. } => 12. + GAP + BARS as f32 * 6. - 3. + GAP + 30.,
            Phase::Working(t) => text(&format!("{t}…")),
            Phase::Message(t) => text(t) + message_icon(t).map_or(0., |_| 14. + 8.),
        };
        (inner + PAD * 2.).min(WINDOW.0 - ROOM * 2.)
    }

    fn recording(&self, raw: bool, since: Instant, ink: &Ink) -> Div {
        let speaking = self.levels.iter().rev().take(4).any(|&l| l > 0.35);
        let dot = if speaking || (self.tick / 8) % 2 == 1 { 10. } else { 8. };
        let secs = since.elapsed().as_secs();
        div()
            .flex()
            .items_center()
            .gap(px(GAP))
            .child(div().size(px(12.)).flex().items_center().justify_center().child(div().size(px(dot)).rounded_full().bg(if raw {
                ink.raw
            } else {
                ink.record
            })))
            .child(div().flex().items_center().gap(px(3.)).h(px(PILL_H)).children(self.levels.iter().map(|&l| {
                div().w(px(3.)).h(px(3. + l.clamp(0., 1.) * 15.)).rounded_full().bg(if l > 0.2 { ink.fg.opacity(0.9) } else { ink.quiet })
            })))
            .child(
                div()
                    .w(px(30.))
                    .text_right()
                    .font_family(look::MONO)
                    .text_size(px(12.))
                    .text_color(ink.dim)
                    .child(format!("{}:{:02}", secs / 60, secs % 60)),
            )
    }
}

impl Render for Pill {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let ink = Ink::of(window);
        let content = match &self.phase {
            Phase::Recording { raw, since } => self.recording(*raw, *since, &ink),
            // Breathes while it works.
            Phase::Working(text) => div().child(
                div().text_color(ink.fg).child(format!("{text}…")).with_animation(
                    "working",
                    Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.55, 1.)),
                    |el, t| el.opacity(t),
                ),
            ),
            Phase::Message(text) => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(ink.fg)
                .when_some(message_icon(text), |d, icon| {
                    d.child(svg().path(SharedString::from(format!("icons/{icon}.svg"))).size(px(14.)).text_color(ink.fg))
                })
                .child(text.clone()),
        };
        let width = self.width(window);
        let shadow = vec![
            BoxShadow { color: ink.highlight, offset: point(px(0.), px(1.)), blur_radius: px(0.), spread_radius: px(0.), inset: true },
            BoxShadow { color: ink.edge, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
            BoxShadow { color: hsla(0., 0., 0., 0.35), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false },
            BoxShadow { color: hsla(0., 0., 0., 0.45), offset: point(px(0.), px(8.)), blur_radius: px(16.), spread_radius: px(-6.), inset: false },
        ];
        let pill = div()
            .h(px(PILL_H))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded_full()
            .bg(ink.bg)
            .shadow(shadow)
            .font_family(look::SANS)
            .text_size(px(TEXT))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .child(div().relative().flex().items_center().child(content).with_animation(
                self.phase.key(),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                |el, t| el.opacity(t).top(px((1. - t) * 6.)),
            ))
            .with_spring("pill-width", spring(SPRING, width), |el, w| el.w(px(w.max(PILL_H))));
        div().size_full().flex().items_end().justify_center().pb(px(ROOM)).child(
            // It rises in when it first shows.
            div().relative().with_animation("enter", Animation::new(Duration::from_millis(320)).with_easing(ease_out_quint()), |el, t| {
                el.opacity(t).top(px((1. - t) * 10.))
            }).child(pill),
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
        point(a.center().x - size.width / 2., a.bottom() - size.height - px(LIFT - ROOM))
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
            margin: Some((px(0.), px(0.), px(LIFT - ROOM), px(0.))),
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
