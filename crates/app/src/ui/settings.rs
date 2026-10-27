//! The settings window: the speech model (downloaded here), the cleanup model
//! (localflow-clean, on this computer), dictation, and the keys to dictate
//! with. Saves to the env file the rest of localflow reads
//! (`localflow_core::Config`).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::Theme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use localflow_core::local::MODEL;
use localflow_core::{Config, Localflow, save_settings};
use localflow_speech::models;

use super::look::{self, Palette};

const SUBMIT: &[(&str, &str)] = &[("When you say \"send it\"", "said"), ("Always", "always"), ("Never", "never")];

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Idle,
    Running(Option<(u64, u64)>),
    Done(String),
    Failed(String),
}

struct State {
    quit_with_settings: bool,
    /// The first-run window instead of the settings, even when set up.
    onboarding: bool,
    window: Option<AnyWindowHandle>,
}

impl Global for State {}

pub fn init(quit_with_settings: bool, onboarding: bool, cx: &mut App) {
    cx.set_global(State { quit_with_settings, onboarding, window: None });
    cx.on_window_closed(|cx, id| {
        let state = cx.global_mut::<State>();
        if state.window.is_some_and(|w| w.window_id() == id) {
            state.window = None;
            if state.quit_with_settings {
                // A download started in the window finishes first.
                super::jobs::Jobs::quit_when_idle(cx);
            }
        }
    })
    .detach();
}

/// Open the settings window, or bring it to the front. The first time (or
/// until the onboarding is finished), the onboarding opens instead.
pub fn open(cx: &mut App) {
    if let Some(window) = cx.global::<State>().window
        && window.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    if cx.global::<State>().onboarding || super::onboarding::needed() {
        match super::onboarding::open(cx) {
            Ok(window) => {
                cx.global_mut::<State>().window = Some(window);
                cx.activate(true);
            }
            Err(e) => eprintln!("onboarding: {e:#}"),
        }
        return;
    }
    let size = size(px(WINDOW.0), px(WINDOW.1));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Localflow".into()),
            // macOS: the traffic lights sit in the window's own top row, by the tabs.
            appears_transparent: cfg!(target_os = "macos"),
            traffic_light_position: Some(point(px(16.), px(16.))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        window_min_size: Some(size),
        is_resizable: false,
        app_id: Some("localflow".into()),
        focus: true,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| SettingsView::new(window, cx))) {
        Ok((window, _)) => {
            cx.global_mut::<State>().window = Some(window);
            cx.activate(true);
        }
        Err(e) => eprintln!("settings: {e:#}"),
    }
}

pub struct SettingsView {
    cfg: Config,
    speech_model: &'static str,
    downloads: Vec<(&'static str, Job, Arc<AtomicBool>)>,
    gpu: bool,
    /// localflow-clean, or the .gguf file the env file names.
    local_model: String,
    /// The cleanup model's download.
    fetch: Job,
    check: Job,
    submit: &'static str,
    history: bool,
    saved: Job,
    tab: Section,
    copied: bool,
}

/// The window's tabs, in the header.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Speech,
    Cleanup,
    Dictation,
    Keys,
}

const WINDOW: (f32, f32) = (760., 620.);
/// The column between the dashed rails.
const COLUMN: f32 = 600.;

/// The value for `value` among `items`, or the first.
fn pick(items: &[(&'static str, &'static str)], value: &str) -> &'static str {
    items.iter().find(|(_, v)| *v == value).map_or(items[0].1, |(_, v)| *v)
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let cfg = Config::from_env();
        let submit = cfg.submit.value().unwrap_or("said");
        let local_model = cfg.model.clone();

        let view = SettingsView {
            speech_model: models::resolve(cfg.speech_model.as_deref()).id,
            downloads: vec![],
            gpu: cfg.speech_gpu,
            local_model,
            fetch: Job::Idle,
            check: Job::Idle,
            submit: pick(SUBMIT, submit),
            history: cfg.history,
            saved: Job::Idle,
            tab: Section::Speech,
            copied: false,
            cfg,
        };
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            look::apply_theme(cx);
        })
        .detach();
        view
    }

    /// The settings as chosen in the window, as env-file changes.
    fn changes(&self) -> Vec<(&'static str, Option<String>)> {
        vec![
            ("LOCALFLOW_MODEL", (self.local_model != MODEL.id).then(|| self.local_model.clone())),
            ("LOCALFLOW_SPEECH_MODEL", Some(self.speech_model.to_string())),
            ("LOCALFLOW_SPEECH_GPU", (!self.gpu).then(|| "off".into())),
            ("LOCALFLOW_CLEANUP_GPU", (!self.gpu).then(|| "off".into())),
            ("LOCALFLOW_SUBMIT", (self.submit != "said").then(|| self.submit.into())),
            ("LOCALFLOW_HISTORY", (!self.history).then(|| "off".into())),
            // Settings localflow no longer has: drop them from an older env file.
            ("LOCALFLOW_API_URL", None),
            ("LOCALFLOW_API_KEY", None),
            ("LOCALFLOW_REVIEW_MODEL", None),
            ("LOCALFLOW_LANGUAGE", None),
            ("LOCALFLOW_ASIDES", None),
            ("LOCALFLOW_LEARN", None),
        ]
    }

    /// The cleanup engine as set up in the window (saved or not), for "Test".
    fn engine(&self) -> Localflow {
        let changes = self.changes();
        let dir = self.cfg.config_dir.clone();
        let get = move |k: &str| -> Option<String> {
            match changes.iter().find(|(name, _)| *name == k) {
                Some((_, value)) => Some(value.clone().unwrap_or_default()),
                None => env_var(k),
            }
        };
        Localflow::new(Config::from_vars(&get, dir))
    }

    /// The local model is here (downloaded, or your .gguf file exists).
    fn local_ready(&self) -> bool {
        Config { model: self.local_model.clone(), ..self.cfg.clone() }.local_model_installed()
    }

    /// Run `work` on its own thread; it reports progress (done, total) and
    /// then its result, which `set` stores in the view.
    fn job<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut dyn FnMut(u64, u64)) -> Result<R, String> + Send + 'static,
        set: impl Fn(&mut SettingsView, Job) + 'static,
        done: impl Fn(R) -> String + 'static,
    ) {
        enum Update<R> {
            Progress(u64, u64),
            Finished(Result<R, String>),
        }
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            let mut last = u64::MAX;
            let progress_tx = tx.clone();
            let mut progress = move |done: u64, total: u64| {
                let step = (done * 200).checked_div(total).unwrap_or(done >> 20);
                if step != last {
                    last = step;
                    let _ = progress_tx.send_blocking(Update::Progress(done, total));
                }
            };
            let _ = tx.send_blocking(Update::Finished(work(&mut progress)));
        });
        set(self, Job::Running(None));
        cx.spawn(async move |this, cx| {
            while let Ok(update) = rx.recv().await {
                let job = match update {
                    Update::Progress(d, t) => Job::Running(Some((d, t))),
                    Update::Finished(Ok(r)) => Job::Done(done(r)),
                    Update::Finished(Err(e)) => Job::Failed(e),
                };
                if this.update(cx, |this, cx| {
                    set(this, job);
                    cx.notify();
                })
                .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn download(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.downloads.retain(|(d, _, _)| *d != id);
        self.downloads.push((id, Job::Idle, cancel.clone()));
        self.job(
            cx,
            move |progress| models::download(id, progress, &cancel).map(|_| ()).map_err(|e| e.to_string()),
            move |this, job| {
                if let Some(d) = this.downloads.iter_mut().find(|(d, _, _)| *d == id) {
                    d.1 = job;
                }
            },
            |_| "Downloaded".into(),
        );
    }

    /// Download the cleanup model.
    fn fetch(&mut self, cx: &mut Context<Self>) {
        let cfg = Config { model: self.local_model.clone(), ..self.cfg.clone() };
        self.job(
            cx,
            move |progress| crate::cleanup_model::get_ready(&cfg, progress, &AtomicBool::new(false)),
            |this, job| this.fetch = job,
            |_| "Downloaded".into(),
        );
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        let lf = self.engine();
        self.job(
            cx,
            move |_| {
                lf.clean("um testing one two three", localflow_core::NO_FILES)
                    .map(|_| format!("Works: {}", lf.cfg.model))
                    .map_err(|e| e.0)
            },
            |this, job| this.check = job,
            |ok| ok,
        );
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes();
        let dir = self.cfg.config_dir.clone();
        let pairs: Vec<(&str, Option<&str>)> = changes.iter().map(|(k, v)| (*k, v.as_deref())).collect();
        self.saved = match save_settings(&dir, &pairs).map_err(|e| e.to_string()).and_then(|_| after_save()) {
            Ok(()) => Job::Done("Saved".into()),
            Err(e) => Job::Failed(format!("Couldn't save: {e}")),
        };
        cx.notify();
    }
}

/// Once saved: the app where it runs from, when it isn't there yet (the
/// Linux AppImage copies itself out, a Mac app adds its login item).
pub(super) fn after_save() -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?.display().to_string();
    let linux = cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some();
    // Not from the disk image or Gatekeeper's temporary copy of a downloaded
    // app (/private/var/.../AppTranslocation/...): neither is there after a restart.
    let mac = cfg!(target_os = "macos") && me.contains(".app/Contents/MacOS/") && !me.starts_with("/Volumes/")
        && !me.contains("/AppTranslocation/");
    if linux || mac {
        crate::setup::install_app(false)?;
    }
    Ok(())
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1e6)
}

fn percent((done, total): (u64, u64)) -> f32 {
    if total == 0 { 0. } else { done as f32 * 100. / total as f32 }
}

/* ------------------------------------------------------------- pieces */

fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// Rows between dashed lines, with a dashed line between each.
fn rows(items: Vec<AnyElement>, p: &Palette) -> Div {
    let rail = p.rail;
    let mut list = div().flex().flex_col().w_full().child(look::dashes(false, rail));
    for item in items {
        list = list.child(item).child(look::dashes(false, rail));
    }
    list
}

/// A setting: what it is (and a hint) on the left, its control on the right.
fn setting(title: &str, hint: Option<&str>, control: impl IntoElement, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .px(px(24.))
        .py(px(14.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.fg).child(title.to_string()))
                .children(hint.map(|h| div().text_size(px(12.)).line_height(px(17.)).text_color(p.dim).child(h.to_string()))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

fn progress_bar(fraction: f32, p: &Palette) -> Div {
    div()
        .relative()
        .w(px(64.))
        .h(px(3.))
        .rounded_full()
        .overflow_hidden()
        .bg(p.muted)
        .child(div().absolute().top_0().left_0().h_full().rounded_full().bg(p.primary).w(relative(fraction.clamp(0., 1.))))
}

fn mono(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().font_family(look::MONO).text_size(px(12.)).text_color(p.dim).child(text.into())
}

/// A job's state in a line: progress, done, or what went wrong.
fn status(job: &Job, p: &Palette) -> Option<Div> {
    match job {
        Job::Idle => None,
        Job::Running(Some(progress)) if progress.1 > 0 => Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(progress_bar(percent(*progress) / 100., p))
                .child(mono(format!("{} of {}", mb(progress.0), mb(progress.1)), p)),
        ),
        Job::Running(_) => Some(div().text_size(px(12.)).text_color(p.dim).child("Working…")),
        Job::Done(text) => {
            Some(div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(p.primary).child(icon("check", 13., p.primary)).child(text.clone()))
        }
        Job::Failed(text) => Some(div().text_size(px(12.)).text_color(p.record).child(text.clone())),
    }
}

/* ------------------------------------------------------------ sections */

impl SettingsView {
    fn speech(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let hover = p.hover;
        let items = models::MODELS
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let id = model.id;
                let selected = self.speech_model == id;
                let job = self.downloads.iter().find(|(d, _, _)| *d == id).map(|(_, j, _)| j.clone());
                let action: AnyElement = if models::installed(id) {
                    mono("Downloaded", p).into_any_element()
                } else if let Some(job @ Job::Running(_)) = &job {
                    status(job, p).map_or_else(|| div().into_any_element(), |d| d.into_any_element())
                } else {
                    look::secondary_button(SharedString::from(format!("get-{id}")), "Download", true, p)
                        .on_click(cx.listener(move |this, _, _, cx| this.download(id, cx)))
                        .into_any_element()
                };
                div()
                    .id(SharedString::from(format!("model-{id}")))
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .px(px(24.))
                    .py(px(10.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(p.tint))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(look::chosen_line(("speech-chosen", i), selected, p))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if selected { p.fg } else { p.fg.opacity(0.75) })
                                            .child(model.name),
                                    )
                                    .child(mono(format!("{} MB", model.size_mb()), p))
                                    .when(model.id == models::default_model().id, |d| {
                                        d.child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.primary).child("Recommended"))
                                    }),
                            )
                            .child(div().text_size(px(12.)).text_color(p.dim).child(model.note))
                            .children(if let Some(Job::Failed(e)) = &job { Some(div().text_size(px(12.)).text_color(p.record).child(e.clone())) } else { None }),
                    )
                    .child(action)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.speech_model = id;
                        if !models::installed(id) && !this.downloads.iter().any(|(d, j, _)| *d == id && matches!(j, Job::Running(_))) {
                            this.download(id, cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let options = look::band(
            vec![
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .w_full()
                    .px(px(24.))
                    .py(px(14.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.fg).child("Use the GPU"))
                            .child(div().text_size(px(12.)).text_color(p.dim).child("Off uses only the CPU")),
                    )
                    .child(Switch::new("gpu").checked(self.gpu).on_click(cx.listener(|this, checked, _, cx| {
                        this.gpu = *checked;
                        cx.notify();
                    })))
                    .into_any_element(),
            ],
            p,
        );
        div().flex().flex_col().gap(px(20.)).pt(px(20.)).child(rows(items, p)).child(options)
    }

    fn cleanup(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let about = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(6.))
            .px(px(24.))
            .py(px(16.))
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(MODEL.id))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(p.dim)
                    .child("localflow's own model, made for cleaning up dictation. It runs on this computer: nothing leaves your machine."),
            )
            .into_any_element();
        let detail = self.local_settings(p, cx);
        let test = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                look::secondary_button("test", if matches!(self.check, Job::Running(_)) { "Testing…" } else { "Test" }, !matches!(self.check, Job::Running(_)), p)
                    .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
            )
            .children(status(&self.check, p));
        div()
            .flex()
            .flex_col()
            .pt(px(20.))
            .child(look::band(vec![about], p))
            .child(div().flex().flex_col().gap(px(14.)).px(px(24.)).pt(px(18.)).child(detail).child(test))
    }

    fn local_settings(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let ready = self.local_ready();
        let ours = self.local_model == MODEL.id;
        let (dot, text) = match (ready, ours) {
            (true, true) => (p.primary, format!("{} is downloaded", MODEL.name)),
            (true, false) => (p.primary, format!("Using {}", self.local_model)),
            (false, true) => (p.faint, format!("{} isn't downloaded yet", MODEL.name)),
            (false, false) => (p.record, format!("{} doesn't exist", self.local_model)),
        };
        let busy = matches!(self.fetch, Job::Running(_));
        let status_line = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(p.fg)
            .child(div().size(px(8.)).rounded_full().bg(dot))
            .child(text)
            .when(!ready && ours, |d| {
                d.child(div().flex_1()).child(
                    look::secondary_button("get-model", if busy { "Downloading…" } else { "Download" }, !busy, p)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !busy {
                                this.fetch(cx);
                            }
                        })),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(status_line)
            .when(ours, |d| d.child(mono(format!("{} MB, runs inside localflow{}", MODEL.size_mb(), if cfg!(any(target_os = "macos", feature = "vulkan")) { ", on the GPU when there is one" } else { ", on the CPU for now" }), p)))
            .children(status(&self.fetch, p))
    }

    fn dictation(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity();
        let submit_options: Vec<(&'static str, SharedString)> =
            SUBMIT.iter().map(|(label, v)| (*v, SharedString::from(if *v == "said" { "When you say “send it”" } else { label }))).collect();
        let submit = look::chips(
            "submit",
            &submit_options,
            self.submit,
            move |v, _, cx| {
                this.update(cx, |this, cx| {
                    this.submit = v;
                    cx.notify();
                })
            },
            p,
            window,
        );
        let stacked = |title: &str, hint: &str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .px(px(24.))
                .py(px(14.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.fg).child(title.to_string()))
                        .child(div().text_size(px(12.)).text_color(p.dim).child(hint.to_string())),
                )
                .child(control)
                .into_any_element()
        };
        div().pt(px(20.)).child(rows(
            vec![
                stacked("Send prompts to coding agents", "Press Enter after pasting into Claude Code, Codex and the like", submit),
                setting(
                    "Keep a history",
                    Some("What you said and what went out, on this computer only"),
                    Switch::new("history").checked(self.history).on_click(cx.listener(|this, checked, _, cx| {
                        this.history = *checked;
                        cx.notify();
                    })),
                    p,
                ),
            ],
            p,
        ))
    }

    fn keys(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        if cfg!(target_os = "linux") {
            let bindings = super::onboarding::bindings(true);
            let copy = bindings.clone();
            return div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .px(px(24.))
                .pt(px(24.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.))
                                .line_height(px(20.))
                                .text_color(p.dim)
                                .child("Your compositor owns the keys, so nothing runs between dictations. On Hyprland, add these to ~/.config/hypr/bindings.lua:"),
                        )
                        .child(look::secondary_button("copy-bindings", if self.copied { "Copied" } else { "Copy" }, true, p).on_click(cx.listener(
                            move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                this.copied = true;
                                cx.notify();
                            },
                        ))),
                )
                .child(
                    div()
                        .rounded(px(10.))
                        .bg(p.muted)
                        .px(px(16.))
                        .py(px(14.))
                        .font_family(look::MONO)
                        .text_size(px(11.))
                        .line_height(px(19.))
                        .text_color(p.fg.opacity(0.8))
                        .overflow_hidden()
                        .children(bindings.lines().map(|l| div().whitespace_nowrap().child(l.to_string()))),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(p.dim)
                        .child("F10 dictates raw (the words as heard), F8 learns from a fix. On other desktops, bind keys to “localflow toggle”, or “localflow start” on press and “localflow stop” on release."),
                );
        }
        let mac = cfg!(target_os = "macos");
        let caps = |keys: &[(&'static str, &'static str, bool)]| {
            div().flex().gap(px(6.)).children(keys.iter().map(|(glyph, label, wide)| look::keycap(glyph, label, *wide, false, p))).into_any_element()
        };
        let (dictate, learn): (look::Keys, look::Keys) = if mac {
            (&[("option", "option", false), ("arrow-big-up", "shift", false), ("", "space", true)], &[("option", "option", false), ("arrow-big-up", "shift", false), ("", "L", false)])
        } else {
            (&[("", "Ctrl", false), ("", "Shift", false), ("", "Space", true)], &[("", "Ctrl", false), ("", "Alt", false), ("", "Space", true)])
        };
        div().pt(px(20.)).child(rows(
            vec![
                setting("Dictate", Some("Hold the keys and talk, or tap them, talk, and tap again"), caps(dictate), p),
                setting("Learn from a fix", Some("Select the sentence you fixed, then press these"), caps(learn), p),
                setting("Cancel", Some("While you dictate: nothing is pasted"), caps(&[("", "esc", false)]), p),
            ],
            p,
        ))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(window);
        let this = cx.entity();
        let tabs = look::chips(
            "tabs",
            &[
                (Section::Speech, "Speech".into()),
                (Section::Cleanup, "Cleanup".into()),
                (Section::Dictation, "Dictation".into()),
                (Section::Keys, "Keys".into()),
            ],
            self.tab,
            move |tab, _, cx| {
                this.update(cx, |this, cx| {
                    this.tab = tab;
                    cx.notify();
                })
            },
            &p,
            window,
        );
        let content = match self.tab {
            Section::Speech => self.speech(&p, cx),
            Section::Cleanup => self.cleanup(&p, cx),
            Section::Dictation => self.dictation(&p, window, cx),
            Section::Keys => self.keys(&p, cx),
        };
        let ready = models::installed(self.speech_model);
        let footer_status = Self::footer_status(&self.saved, ready, &p);
        let rail = p.rail;
        let tab_index = self.tab as usize;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family(look::SANS)
            .text_size(px(14.))
            .child(div().flex_none().h(px(48.)).flex().items_center().justify_center().child(tabs))
            .child(look::dashes(false, rail))
            .child(
                div().flex_1().min_h_0().flex().justify_center().overflow_hidden().child(
                    div()
                        .relative()
                        .w(px(COLUMN))
                        .h_full()
                        .child(div().absolute().top_0().bottom_0().left_0().child(look::dashes(true, rail)))
                        .child(div().absolute().top_0().bottom_0().right_0().child(look::dashes(true, rail)))
                        .child(
                            div()
                                .id("settings-scroll")
                                .relative()
                                .size_full()
                                .overflow_y_scroll()
                                .child(content)
                                .with_animation(
                                    ("settings-tab", tab_index),
                                    Animation::new(std::time::Duration::from_millis(320)).with_easing(ease_out_quint()),
                                    |el, t| el.opacity(t).top(px((1. - t) * 6.)),
                                ),
                        ),
                ),
            )
            .child(look::dashes(false, rail))
            .child(
                div()
                    .flex_none()
                    .h(px(64.))
                    .px(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(footer_status))
                    .child(look::primary_button("save", "Save", true, &p).on_click(cx.listener(|this, _, _, cx| this.save(cx)))),
            )
    }
}

impl SettingsView {
    fn footer_status(saved: &Job, ready: bool, p: &Palette) -> Div {
        if let Some(line) = status(saved, p) {
            return line;
        }
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(p.dim)
            .child(div().size(px(7.)).rounded_full().bg(if ready { p.primary } else { p.record }))
            .child(if ready { "Speech model ready" } else { "Download a speech model" })
    }
}
