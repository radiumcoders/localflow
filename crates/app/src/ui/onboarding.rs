//! The first-run window, in the look of localflow.radiumcoders.com: pick a
//! speech model (it downloads while you go on), where cleanup runs, allow
//! what macOS asks for, bind the keys (Linux), try a dictation for real, done.
//!
//! It opens instead of the settings until you finish it (`localflow
//! onboarding` opens it again). Leaving the cleanup step saves the settings,
//! so the hotkey already works when you try it.

use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use localflow_core::{Config, Localflow, Msg, config_dir, prompts::HAIKU, save_api_key, save_settings};
use localflow_speech::models;

use super::jobs::{Job, Jobs};
use crate::ollama;
use super::look::{self, Palette, SOFT, SPRING, band, chips, dashes, dot_grid, ghost_button, primary_button, secondary_button, spring};
use super::settings::{LOCAL_MODELS, after_save, env_file_value};

const WINDOW: (f32, f32) = (760., 564.);
/// The column between the dashed rails.
const COLUMN: f32 = 600.;
/// In the config folder while the onboarding isn't finished (it was saved
/// part way), so it opens again next time.
const MARKER: &str = "onboarding";

/// Nothing set up yet, or an onboarding left part way.
pub fn needed() -> bool {
    let dir = config_dir();
    !dir.join("env").exists() || dir.join(MARKER).exists()
}

pub fn open(cx: &mut App) -> anyhow::Result<AnyWindowHandle> {
    let size = size(px(WINDOW.0), px(WINDOW.1));
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Localflow".into()),
            // macOS: the traffic lights sit in the window's own top row.
            appears_transparent: cfg!(target_os = "macos"),
            traffic_light_position: Some(point(px(16.), px(16.))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        window_min_size: Some(size),
        // A fixed size also makes tiling compositors (Hyprland) float it.
        is_resizable: false,
        app_id: Some("localflow".into()),
        focus: true,
        ..Default::default()
    };
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Onboarding::new(window, cx)))?;
    Ok(window)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    /// This computer's; a debug build can preview another's flow with
    /// LOCALFLOW_PREVIEW_OS=macos|windows|linux.
    fn current() -> Platform {
        if cfg!(debug_assertions) {
            match std::env::var("LOCALFLOW_PREVIEW_OS").as_deref() {
                Ok("macos") => return Platform::Mac,
                Ok("windows") => return Platform::Windows,
                Ok("linux") => return Platform::Linux,
                _ => {}
            }
        }
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }

    fn steps(self) -> &'static [Step] {
        use Step::*;
        match self {
            Platform::Mac => &[Welcome, Speech, Cleanup, Permissions, Try, Done],
            Platform::Windows => &[Welcome, Speech, Cleanup, Try, Done],
            Platform::Linux => &[Welcome, Speech, Cleanup, Keys, Try, Done],
        }
    }

    /// The hotkey, as keycaps: (icon, label, wide).
    fn keys(self) -> &'static [(&'static str, &'static str, bool)] {
        match self {
            Platform::Mac => &[("option", "option", false), ("arrow-big-up", "shift", false), ("", "space", true)],
            Platform::Windows => &[("", "Ctrl", false), ("", "Shift", false), ("", "Space", true)],
            Platform::Linux => &[("", "F9", false)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Welcome,
    Speech,
    Cleanup,
    Permissions,
    Keys,
    Try,
    Done,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::Speech => "Speech",
            Step::Cleanup => "Cleanup",
            Step::Permissions => "Permissions",
            Step::Keys => "Keys",
            Step::Try => "Try it",
            Step::Done => "Done",
        }
    }
}

/// A speech model as the onboarding offers it (the settings list them all).
struct Pick {
    id: &'static str,
    name: &'static str,
    body: &'static str,
}

const ENGLISH: [Pick; 3] = [
    Pick { id: "base.en", name: "Base", body: "The quickest to load. Good for short notes." },
    Pick { id: "small.en", name: "Small", body: "Fast on any recent computer. Spells names right." },
    Pick { id: "large-v3-turbo", name: "Large", body: "The most accurate. Best with a GPU." },
];
const ANY_LANGUAGE: [Pick; 3] = [
    Pick { id: "base", name: "Base", body: "The quickest to load. Good for short notes." },
    Pick { id: "small", name: "Small", body: "A good balance of speed and accuracy." },
    Pick { id: "large-v3-turbo", name: "Large", body: "The most accurate in every language. Best with a GPU." },
];
const RECOMMENDED: usize = 1;

/// Hosted providers offered here, by their LOCALFLOW_API_URL name; "" takes
/// any OpenAI-compatible base URL. The settings have the rest.
const PROVIDERS: &[(&str, &str)] =
    &[("Anthropic", "anthropic"), ("OpenAI", "openai"), ("OpenRouter", "openrouter"), ("Groq", "groq"), ("Gemini", "gemini"), ("Other", "")];

#[derive(Clone, Debug, PartialEq)]
enum Ollama {
    Checking,
    Down,
    Up(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
enum Check {
    Idle,
    Running,
    Ok(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Mic {
    Unasked,
    Asking,
    Allowed,
    Failed(String),
}

fn speech_key(id: &str) -> String {
    format!("speech:{id}")
}

fn pull_key(model: &str) -> String {
    format!("ollama:{model}")
}

/// What getting Ollama ready is doing, as a line.
fn job_label(job: &Job, model: &str) -> String {
    match job {
        Job::Running(_, _, stage) if !stage.is_empty() && *stage != ollama::PULLING => stage.to_string(),
        _ => format!("Downloading {model}"),
    }
}

/// Getting Ollama ready: a bar, the step, how far.
fn job_progress(job: &Job, model: &str, p: &Palette) -> Div {
    let known = matches!(job, Job::Running(_, total, _) if *total > 0);
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .text_size(px(13.))
        .text_color(p.dim)
        .child(progress_bar(job.fraction(), p))
        .child(job_label(job, model))
        .when(known, |d| d.child(mono(format!("{}%", (job.fraction() * 100.) as u32), p)))
}

fn has_model(installed: &[String], model: &str) -> bool {
    installed.iter().any(|m| m == model || m.strip_suffix(":latest") == Some(model))
}

/// A provider's error in a line: what went wrong, without the JSON.
fn plain_error(e: &str) -> String {
    if e.contains("401") || e.contains("403") {
        return "The provider didn't accept that key.".into();
    }
    if e.contains("404") {
        return "The provider doesn't know that model.".into();
    }
    let line = e.lines().next().unwrap_or(e).trim().trim_end_matches('{').trim();
    match line.char_indices().nth(90) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

fn me() -> std::path::PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "localflow".into())
}

pub struct Onboarding {
    cfg: Config,
    platform: Platform,
    index: usize,
    /// 1 going forward, -1 going back: where the next step slides in from.
    direction: f32,
    english: bool,
    speech: &'static str,
    api: bool,
    local_model: &'static str,
    ollama: Ollama,
    provider: usize,
    api_model: Entity<InputState>,
    api_url: Entity<InputState>,
    api_key: Entity<InputState>,
    has_key: bool,
    check: Check,
    mic: Mic,
    accessibility: bool,
    try_field: Entity<TextareaState>,
    focus_try: bool,
    holding: bool,
    /// Something was dictated into the field.
    heard: bool,
    copied: bool,
    error: Option<String>,
}

impl Onboarding {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Onboarding {
        let cfg = Config::from_env();
        let speech = models::resolve(cfg.speech_model.as_deref()).id;
        let english = !matches!(speech, "base" | "small");
        let has_key = cfg.api_key.is_some() || cfg.key_file().exists();
        // Opened again: start from what's saved.
        let saved_url = std::env::var("LOCALFLOW_API_URL").ok().or_else(|| env_file_value(&cfg, "LOCALFLOW_API_URL")).unwrap_or_default();
        let saved_url = saved_url.trim().to_lowercase();
        let api = !saved_url.is_empty();
        let provider = if api { PROVIDERS.iter().position(|(_, id)| *id == saved_url).unwrap_or(PROVIDERS.len() - 1) } else { 0 };
        let custom_url = if api && PROVIDERS[provider].1.is_empty() { cfg.api_url.clone() } else { String::new() };
        let api_model = if api && PROVIDERS[provider].1 != "anthropic" { cfg.model.clone() } else { String::new() };
        let local_model = if api { LOCAL_MODELS[0] } else { LOCAL_MODELS.iter().copied().find(|m| *m == cfg.model).unwrap_or(LOCAL_MODELS[0]) };
        let try_field = cx.new(|cx| TextareaState::new(window, cx).placeholder("Your words land here."));
        cx.subscribe(&try_field, |this, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) && !field.read(cx).value().trim().is_empty() {
                this.heard = true;
                cx.notify();
            }
        })
        .detach();
        cx.observe_global::<Jobs>(|this, cx| {
            // Once Ollama has the model, list its models again.
            if Jobs::get(&pull_key(this.local_model), cx) == Some(Job::Done)
                && !matches!(&this.ollama, Ollama::Up(m) if has_model(m, this.local_model))
            {
                this.check_ollama(cx);
            }
            cx.notify();
        })
        .detach();
        // macOS's Accessibility switch is flipped in System Settings: look again every second.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let allowed = localflow_desktop::accessibility_allowed();
                let alive = this.update(cx, |this, cx| {
                    if this.accessibility != allowed {
                        this.accessibility = allowed;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut view = Onboarding {
            platform: Platform::current(),
            index: 0,
            direction: 1.,
            english,
            speech,
            api,
            local_model,
            ollama: Ollama::Checking,
            provider,
            api_model: cx.new(|cx| InputState::new(window, cx).placeholder("Model name").default_value(api_model)),
            api_url: cx.new(|cx| InputState::new(window, cx).placeholder("https://my-server.example/v1").default_value(custom_url)),
            api_key: cx.new(|cx| {
                InputState::new(window, cx).placeholder(if has_key { "Saved (type to replace)" } else { "API key" }).masked(true)
            }),
            has_key,
            check: Check::Idle,
            mic: Mic::Unasked,
            accessibility: localflow_desktop::accessibility_allowed(),
            try_field,
            focus_try: false,
            holding: false,
            heard: false,
            copied: false,
            error: None,
            cfg,
        };
        for input in [view.api_model.clone(), view.api_url.clone(), view.api_key.clone()] {
            cx.subscribe(&input, |this, _, _: &InputEvent, cx| {
                this.check = Check::Idle;
                cx.notify();
            })
            .detach();
        }
        view.check_ollama(cx);
        view
    }

    fn step(&self) -> Step {
        self.platform.steps()[self.index]
    }

    fn picks(&self) -> &'static [Pick; 3] {
        if self.english { &ENGLISH } else { &ANY_LANGUAGE }
    }

    fn pick(&self) -> &'static Pick {
        self.picks().iter().find(|p| p.id == self.speech).unwrap_or(&self.picks()[RECOMMENDED])
    }

    fn speech_job(&self, cx: &App) -> Option<Job> {
        Jobs::get(&speech_key(self.pick().id), cx)
    }

    fn speech_ready(&self) -> bool {
        models::installed(self.pick().id)
    }

    fn provider_id(&self) -> &'static str {
        PROVIDERS[self.provider].1
    }

    fn local_ready(&self) -> bool {
        matches!(&self.ollama, Ollama::Up(installed) if has_model(installed, self.local_model))
    }

    fn cleanup_job(&self, cx: &App) -> Option<Job> {
        if self.api { None } else { Jobs::get(&pull_key(self.local_model), cx) }
    }

    /// Install Ollama if it isn't here, start it, and pull the model: on
    /// a thread of its own, going on after the window closes.
    fn get_ollama(&mut self, cx: &mut Context<Self>) {
        let model = self.local_model;
        let lf = Localflow::new(self.cfg.clone());
        Jobs::start_staged(pull_key(model), cx, move |report| ollama::get_ready(&lf, model, report));
        cx.notify();
    }

    fn check_ollama(&mut self, cx: &mut Context<Self>) {
        let lf = Localflow::new(self.cfg.clone());
        let task = cx.background_executor().spawn(async move { lf.ollama_models() });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ollama = match found {
                    Ok(models) => Ollama::Up(models),
                    Err(_) => Ollama::Down,
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// The settings as chosen, as env-file changes.
    fn changes(&self, cx: &App) -> Vec<(&'static str, Option<String>)> {
        let mut changes: Vec<(&'static str, Option<String>)> = vec![];
        if self.api {
            let id = self.provider_id();
            let url = if id.is_empty() { self.api_url.read(cx).value().trim().to_string() } else { id.to_string() };
            let model = if id == "anthropic" { HAIKU.to_string() } else { self.api_model.read(cx).value().trim().to_string() };
            changes.push(("LOCALFLOW_API_URL", Some(url)));
            changes.push(("LOCALFLOW_MODEL", Some(model).filter(|m| !m.is_empty())));
        } else {
            changes.push(("LOCALFLOW_API_URL", None));
            changes.push(("LOCALFLOW_MODEL", Some(self.local_model.to_string())));
        }
        changes.push(("LOCALFLOW_SPEECH_MODEL", Some(self.pick().id.to_string())));
        changes
    }

    fn typed_key(&self, cx: &App) -> String {
        self.api_key.read(cx).value().trim().to_string()
    }

    /// Save what's chosen so far, and mark the onboarding as not finished.
    fn save(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let changes = self.changes(cx);
        let dir = self.cfg.config_dir.clone();
        let key = self.typed_key(cx);
        let pairs: Vec<(&str, Option<&str>)> = changes.iter().map(|(k, v)| (*k, v.as_deref())).collect();
        save_settings(&dir, &pairs).map_err(|e| e.to_string())?;
        if self.api && !key.is_empty() {
            save_api_key(&dir, &key).map_err(|e| e.to_string())?;
            self.has_key = true;
        }
        std::fs::write(dir.join(MARKER), "").map_err(|e| e.to_string())?;
        after_save()
    }

    /// Ask the cleanup model for one word, with the settings as typed.
    fn test(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes(cx);
        let key = self.typed_key(cx);
        let dir = self.cfg.config_dir.clone();
        let get = move |k: &str| -> Option<String> {
            if k == "LOCALFLOW_API_KEY" && !key.is_empty() {
                return Some(key.clone());
            }
            match changes.iter().find(|(name, _)| *name == k) {
                Some((_, value)) => Some(value.clone().unwrap_or_default()),
                None => std::env::var(k).ok(),
            }
        };
        let lf = Localflow::new(Config::from_vars(&get, dir));
        self.check = Check::Running;
        let task = cx.background_executor().spawn(async move {
            let model = lf.cfg.model.clone();
            lf.chat(&model, &[Msg::user("Reply with just: ok")], 32, None).map(|_| format!("It works: {model}")).map_err(|e| e.0)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.check = match result {
                    Ok(text) => Check::Ok(text),
                    Err(e) => Check::Failed(plain_error(&e)),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn ask_mic(&mut self, cx: &mut Context<Self>) {
        self.mic = Mic::Asking;
        // Opening the microphone is what makes macOS ask.
        let task = cx.background_executor().spawn(async move {
            localflow_speech::Recorder::start(|_| {}).map(|recorder| {
                std::thread::sleep(Duration::from_millis(400));
                recorder.stop();
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.mic = match result {
                    Ok(()) => Mic::Allowed,
                    Err(e) => Mic::Failed(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn can_continue(&self, cx: &App) -> bool {
        match self.step() {
            Step::Cleanup if self.api => {
                let id = self.provider_id();
                let key = self.has_key || !self.typed_key(cx).is_empty();
                let model = id == "anthropic" || !self.api_model.read(cx).value().trim().is_empty();
                let url = !id.is_empty() || !self.api_url.read(cx).value().trim().is_empty();
                key && model && url
            }
            // Continue installs what's missing.
            Step::Cleanup => self.ollama != Ollama::Checking,
            _ => true,
        }
    }

    fn go(&mut self, to: usize, cx: &mut Context<Self>) {
        let to = to.min(self.platform.steps().len() - 1);
        self.direction = if to >= self.index { 1. } else { -1. };
        self.index = to;
        self.error = None;
        self.focus_try = self.step() == Step::Try;
        cx.notify();
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_continue(cx) {
            return;
        }
        match self.step() {
            Step::Speech => {
                let id = self.pick().id;
                if !models::installed(id) {
                    Jobs::start(speech_key(id), cx, move |progress| {
                        models::download(id, progress, &AtomicBool::new(false)).map(|_| ()).map_err(|e| e.to_string())
                    });
                }
            }
            Step::Cleanup => {
                if let Err(e) = self.save(cx) {
                    self.error = Some(format!("Couldn't save the settings: {e}"));
                    cx.notify();
                    return;
                }
                if !self.api && !self.local_ready() && !matches!(self.cleanup_job(cx), Some(Job::Running(..) | Job::Done)) {
                    self.get_ollama(cx);
                }
            }
            Step::Done => return self.finish(window, cx),
            _ => {}
        }
        self.go(self.index + 1, cx);
    }

    /// Save once more (you may have gone back and changed something), and close.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.save(cx) {
            Ok(()) => {
                let _ = std::fs::remove_file(self.cfg.config_dir.join(MARKER));
                window.remove_window();
            }
            Err(e) => {
                self.error = Some(format!("Couldn't save the settings: {e}"));
                cx.notify();
            }
        }
    }

    /// Hold-to-talk on the keycaps: the same as holding the hotkey.
    fn press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.holding || !self.speech_ready() {
            return;
        }
        self.try_field.update(cx, |field, cx| field.focus(window, cx));
        self.holding = true;
        run(&["start"]);
        cx.notify();
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if !self.holding {
            return;
        }
        self.holding = false;
        run(&["stop"]);
        cx.notify();
    }
}

/// Run localflow with `args`, without waiting.
fn run(args: &[&str]) {
    if let Err(e) = Command::new(me()).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        eprintln!("onboarding: couldn't run localflow {}: {e}", args.join(" "));
    }
}

/// Linux: the Hyprland key bindings, short enough to read in a window.
/// `all` adds raw dictation (F10) and learning from a fix (F8).
pub(super) fn bindings(all: bool) -> String {
    let exe = crate::setup::app_exe().display().to_string();
    let exe = if exe.contains(' ') { format!("'{exe}'") } else { exe };
    let mut lines = vec![
        format!("local lf = \"{exe}\""),
        "hl.unbind(\"SUPER + CTRL + X\")".to_string(),
        "hl.unbind(\"F9\")".to_string(),
        "o.bind(\"SUPER + CTRL + X\", \"Toggle dictation\", lf .. \" toggle\")".to_string(),
        "o.bind(\"F9\", \"Start dictation\", lf .. \" start\")".to_string(),
        "o.bind(\"F9\", \"Stop dictation\", lf .. \" stop\", { release = true })".to_string(),
    ];
    if all {
        lines.extend([
            "o.bind(\"F10\", \"Start raw dictation\", lf .. \" start raw\")".to_string(),
            "o.bind(\"F10\", \"Stop raw dictation\", lf .. \" stop\", { release = true })".to_string(),
            "o.bind(\"F8\", \"Learn from a fix\", lf .. \" learn\")".to_string(),
        ]);
    }
    lines.join("\n")
}

/* ------------------------------------------------------------- pieces */

fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// The heading a step opens with, as far from the line above as from what follows.
fn head(title: &str, body: impl IntoElement, p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .px(px(40.))
        .pt(px(36.))
        .pb(px(28.))
        .child(div().text_size(px(28.)).line_height(px(34.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(title.to_string()))
        .child(div().max_w(px(448.)).text_center().text_size(px(15.)).line_height(px(24.)).text_color(p.dim).child(body))
}

fn note(icon_name: &str, text: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(p.dim)
        .child(icon(icon_name, 12., p.dim))
        .child(text.into())
}

/// A tonal, borderless field around an input.
fn field(input: impl IntoElement, p: &Palette) -> Div {
    div().h(px(36.)).flex().items_center().px(px(16.)).rounded_full().bg(p.muted).text_size(px(14.)).child(input)
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

/* -------------------------------------------------------------- steps */

impl Onboarding {
    /// A cell you pick, one of a group: the blue line runs along its top.
    fn option(
        &self,
        id: (&'static str, usize),
        checked: bool,
        children: Vec<AnyElement>,
        on_pick: impl Fn(&mut Onboarding, &mut Context<Onboarding>) + 'static,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = p.hover;
        div()
            .id(id)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(8.))
            .px(px(24.))
            .py(px(20.))
            .cursor_pointer()
            .when(checked, |d| d.bg(p.tint))
            .when(!checked, |d| d.hover(move |s| s.bg(hover)))
            .child(look::chosen_line((id.0, id.1 + 100), checked, p))
            .children(children)
            .on_click(cx.listener(move |this, _, _, cx| {
                on_pick(this, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    fn welcome(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let palette = *p;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(34.))
            .pb(px(24.))
            .child(dot_grid(p.rail))
            .child(div().size(px(96.)).with_animation(
                "logo",
                Animation::new(Duration::from_millis(1100)),
                move |el, t| el.child(look::logo(96., t, &palette)),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div().text_size(px(40.)).line_height(px(44.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child("Welcome to Localflow"),
                    )
                    .child(
                        div()
                            .max_w(px(384.))
                            .text_center()
                            .text_size(px(15.))
                            .line_height(px(24.))
                            .text_color(p.dim)
                            .child("Talk, and get clean text where you type. Three quick choices and you're set."),
                    )
                    .relative()
                    .with_animation("welcome-text", Animation::new(Duration::from_millis(900)).with_easing(ease_out_quint()), |el, t| {
                        let t = ((t - 0.45) / 0.55).clamp(0., 1.);
                        el.opacity(t).top(px((1. - t) * 8.))
                    })

            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(20.))
                    .child(primary_button("start", "Get started", true, p).on_click(cx.listener(|this, _, window, cx| this.next(window, cx))))
                    .child(note("lock", "Runs on your computer. Nothing leaves it unless you add an API key.", p))
                    .relative()
                    .with_animation("welcome-actions", Animation::new(Duration::from_millis(1000)).with_easing(ease_out_quint()), |el, t| {
                        let t = ((t - 0.55) / 0.45).clamp(0., 1.);
                        el.opacity(t).top(px((1. - t) * 8.))
                    })

            )
    }

    fn speech(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity();
        let language = chips(
            "language",
            &[(true, "English".into()), (false, "Other languages".into())],
            self.english,
            move |english, _, cx| {
                this.update(cx, |this, cx| {
                    if this.english != english {
                        this.english = english;
                        this.speech = this.picks()[RECOMMENDED].id;
                        cx.notify();
                    }
                })
            },
            p,
            window,
        );
        let chosen = self.pick().id;
        let cells = self
            .picks()
            .iter()
            .enumerate()
            .map(|(i, pick)| {
                let checked = pick.id == chosen;
                let size = models::find(pick.id).map_or(0, |m| m.size_mb());
                let id = pick.id;
                self.option(
                    ("model", i),
                    checked,
                    vec![
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if checked { p.fg } else { p.fg.opacity(0.7) })
                                    .child(pick.name),
                            )
                            .when(i == RECOMMENDED, |d| {
                                d.child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(p.primary).child("Recommended"))
                            })
                            .into_any_element(),
                        div().text_size(px(13.)).line_height(px(18.)).text_color(p.dim).child(pick.body).into_any_element(),
                        div()
                            .mt_auto()
                            .pt(px(4.))
                            .child(mono(if models::installed(id) { "Downloaded".to_string() } else { format!("{size} MB") }, p))
                            .into_any_element(),
                    ],
                    move |this, _| this.speech = id,
                    p,
                    cx,
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(head("How it hears you", "A speech model turns your voice into words, right here on this computer. Pick one; it downloads while you finish setting up.", p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(12.))
                            .child(div().text_size(px(14.)).text_color(p.dim).child("You speak"))
                            .child(language),
                    )
                    .child(band(cells, p))
                    .child(note("cpu", "Runs on your GPU when there is one, and loads only while it transcribes.", p)),
            )
    }

    fn cleanup(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let choices = [
            (false, "On this computer", "Private: nothing leaves your machine. Runs with Ollama."),
            (true, "With your API key", "Nothing to download. Each dictation goes to the provider you pick."),
        ];
        let cells = choices
            .iter()
            .enumerate()
            .map(|(i, (api, title, body))| {
                let (api, checked) = (*api, self.api == *api);
                self.option(
                    ("cleanup", i),
                    checked,
                    vec![
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if checked { p.fg } else { p.fg.opacity(0.7) })
                            .child(*title)
                            .into_any_element(),
                        div().text_size(px(13.)).line_height(px(18.)).text_color(p.dim).child(*body).into_any_element(),
                    ],
                    move |this, _| {
                        this.api = api;
                        this.check = Check::Idle;
                    },
                    p,
                    cx,
                )
            })
            .collect();
        let detail = if self.api { self.api_detail(p, window, cx) } else { self.local_detail(p, window, cx) };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(head(
                "Where cleanup runs",
                "A small language model turns what you said into what you meant: no fillers, no changes of mind, proper punctuation.",
                p,
            ))
            .child(band(cells, p))
            .child(
                div()
                    .id(("detail", self.api as usize))
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(14.))
                    .px(px(32.))
                    .pt(px(22.))
                    .child(detail)
                    .with_animation(("detail-in", self.api as usize), Animation::new(Duration::from_millis(350)).with_easing(ease_out_quint()), |el, t| {
                        el.opacity(t).top(px((1. - t) * 6.))
                    }),
            )
    }

    fn local_detail(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let (dot, text) = match &self.ollama {
            Ollama::Checking => (p.faint, "Looking for Ollama…"),
            Ollama::Down if !ollama::is_installed() => (p.faint, "Ollama isn't installed yet"),
            Ollama::Down => (p.record, "Ollama isn't running"),
            Ollama::Up(_) => (p.primary, "Ollama is running"),
        };
        let status = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(14.))
            .text_color(if matches!(self.ollama, Ollama::Up(_)) { p.fg } else { p.dim })
            .child(div().size(px(8.)).rounded_full().bg(dot))
            .child(text);
        let mut col = div().flex().flex_col().items_center().gap(px(14.)).child(status);
        if self.ollama == Ollama::Checking {
            return col;
        }
        let this = cx.entity();
        let options: Vec<(&'static str, SharedString)> = LOCAL_MODELS.iter().map(|m| (*m, SharedString::from(*m))).collect();
        col = col.child(chips(
            "local-model",
            &options,
            self.local_model,
            move |m, _, cx| {
                this.update(cx, |this, cx| {
                    this.local_model = m;
                    cx.notify();
                })
            },
            p,
            window,
        ));
        let model = self.local_model;
        let size = if model == LOCAL_MODELS[0] { " (3.3 GB)" } else { "" };
        let note = |text: String| div().max_w(px(520.)).text_center().text_size(px(13.)).line_height(px(18.)).text_color(p.dim).child(text);
        let action = if self.local_ready() {
            div().flex().items_center().gap(px(6.)).text_size(px(13.)).text_color(p.primary).child(icon("check", 14., p.primary)).child("Installed")
        } else {
            match Jobs::get(&pull_key(model), cx) {
                Some(job @ Job::Running(..)) => job_progress(&job, model, p),
                Some(Job::Failed(e)) => div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(div().max_w(px(420.)).text_center().text_size(px(12.)).text_color(p.record).child(format!("Couldn't set it up: {e}")))
                    .child(secondary_button("retry-ollama", "Try again", true, p).on_click(cx.listener(|this, _, _, cx| this.get_ollama(cx)))),
                _ if matches!(self.ollama, Ollama::Up(_)) => note(format!("Continue downloads {model}{size} while you go on.")),
                _ if ollama::is_installed() => note(format!("Continue starts Ollama and downloads {model}{size}.")),
                _ => note(format!("Continue installs Ollama and {model}{size} while you go on.")),
            }
        };
        col.child(action).child(
            div()
                .max_w(px(400.))
                .text_center()
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(p.dim)
                .child("4b is the most careful with long dictation; 2b is quicker on computers without a GPU."),
        )
    }

    fn api_detail(&self, p: &Palette, window: &Window, cx: &mut Context<Self>) -> Div {
        let this = cx.entity();
        let options: Vec<(usize, SharedString)> = PROVIDERS.iter().enumerate().map(|(i, (label, _))| (i, SharedString::from(*label))).collect();
        let providers = chips(
            "provider",
            &options,
            self.provider,
            move |i, _, cx| {
                this.update(cx, |this, cx| {
                    this.provider = i;
                    this.check = Check::Idle;
                    cx.notify();
                })
            },
            p,
            window,
        );
        let id = self.provider_id();
        let result = match &self.check {
            Check::Idle => None,
            Check::Running => Some(div().text_color(p.dim).child("Asking…")),
            Check::Ok(text) => Some(div().flex().items_center().gap(px(6.)).text_color(p.primary).child(icon("check", 13., p.primary)).child(text.clone())),
            Check::Failed(e) => Some(div().text_color(p.record).child(e.clone())),
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .w_full()
            .child(providers)
            .when(id.is_empty(), |d| d.child(field(Input::new(&self.api_url).appearance(false), p).w(px(440.))))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .w(px(440.))
                    .when(id != "anthropic", |d| d.child(field(Input::new(&self.api_model).appearance(false), p).w(px(170.))))
                    .child(field(Input::new(&self.api_key).appearance(false), p).flex_1())
                    .child(
                        secondary_button("test", "Test", self.check != Check::Running, p)
                            .h(px(36.))
                            .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
                    ),
            )
            .child(
                div()
                    .h(px(18.))
                    .max_w(px(440.))
                    .overflow_hidden()
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .children(result)
                    .when(self.check == Check::Idle, |d| {
                        d.child(note(
                            "lock",
                            if id == "anthropic" { "Saved where only you can read it. Uses Claude Haiku." } else { "Saved where only you can read it." },
                            p,
                        ))
                    }),
            )
    }

    fn permissions(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let items = [
            ("mic", "mic", "Microphone", "To hear you, only while you hold the keys.", self.mic == Mic::Allowed),
            ("access", "person-standing", "Accessibility", "For the hotkey, and to paste where you were typing.", self.accessibility),
        ];
        let cells = items
            .into_iter()
            .map(|(id, icon_name, title, body, on)| {
                let action = if on {
                    div().h(px(32.)).flex().items_center().gap(px(6.)).text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(p.primary).child(icon("check", 15., p.primary)).child("Allowed")
                } else if id == "mic" {
                    let asking = self.mic == Mic::Asking;
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().flex().child(secondary_button("allow-mic", if asking { "Waiting for macOS…" } else { "Allow" }, !asking, p).when(!asking, |b| {
                            b.on_click(cx.listener(|this, _, _, cx| this.ask_mic(cx)))
                        })))
                        .when_some(if let Mic::Failed(e) = &self.mic { Some(e.clone()) } else { None }, |d, e| {
                            d.child(div().text_size(px(12.)).text_color(p.record).child(e))
                        })
                } else {
                    div().flex().child(secondary_button("allow-access", "Allow", true, p).on_click(|_, _, _| localflow_desktop::ask_accessibility()))
                };
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(18.))
                    .px(px(24.))
                    .py(px(24.))
                    .child(look::chosen_line(("perm", id.len()), on, p))
                    .child(
                        div()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(if on { p.tint_strong } else { p.muted })
                            .child(icon(icon_name, 16., if on { p.primary } else { p.fg.opacity(0.7) })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(title))
                            .child(div().h(px(36.)).text_size(px(13.)).line_height(px(18.)).text_color(p.dim).child(body)),
                    )
                    .child(action)
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(head("Two permissions", "macOS asks for these once. Localflow uses them only while you dictate, and never records in the background.", p))
            .child(band(cells, p))
    }


    fn keys(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let text = bindings(false);
        let copy = text.clone();
        div()
            .flex()
            .flex_col()
            .child(head(
                "Bind your keys",
                "Your compositor owns the keys, so nothing runs between dictations. On Hyprland, add these to ~/.config/hypr/bindings.lua.",
                p,
            ))
            .child(
                div().px(px(32.)).child(
                    div()
                        .relative()
                        .rounded(px(10.))
                        .bg(p.muted)
                        .pl(px(16.))
                        .pr(px(88.))
                        .py(px(14.))
                        .font_family(look::MONO)
                        .text_size(px(11.5))
                        .line_height(px(20.))
                        .text_color(p.fg.opacity(0.8))
                        .children(text.lines().map(|l| div().whitespace_nowrap().child(l.to_string())))
                        .child(
                            div().absolute().top(px(10.)).right(px(10.)).child(
                                secondary_button("copy", if self.copied { "Copied" } else { "Copy" }, true, p).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                        this.copied = true;
                                        cx.notify();
                                    },
                                )),
                            ),
                        ),
                ),
            )
            .child(
                div()
                    .pt(px(18.))
                    .px(px(40.))
                    .text_center()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(p.dim)
                    .child("On other desktops, bind a key to “localflow toggle”, or “localflow start” on press and “localflow stop” on release."),
            )
    }

    fn try_it(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let ready = self.speech_ready();
        let held = self.holding;
        let body = match self.platform {
            Platform::Linux => "Hold F9, say something, and let go. Or hold the key below.",
            _ => "Hold the keys, say something, and let go. Or tap them, talk, and tap again.",
        };
        let keycaps = div()
            .id("keycaps")
            .flex()
            .gap(px(6.))
            .when(ready, |d| d.cursor_pointer())
            .when(!ready, |d| d.opacity(0.5))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.press(window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.release(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.release(cx)))
            .children(self.platform.keys().iter().map(|(glyph, label, wide)| look::keycap(glyph, label, *wide, held, p)));
        let hint = if ready {
            None
        } else {
            let job = self.speech_job(cx);
            Some(match job {
                Some(Job::Failed(e)) => div().text_color(p.record).child(format!("The speech model didn't download: {e}")),
                Some(job) => div().text_color(p.dim).child(format!("The speech model is still downloading · {}%", (job.fraction() * 100.) as u32)),
                None => div().text_color(p.dim).child("Go back and pick a speech model first."),
            })
        };
        div()
            .flex()
            .flex_col()
            .child(head("Try it", body, p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(24.))
                    .child(keycaps)
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .child(dashes(false, p.rail))
                            .child(
                                div()
                                    .relative()
                                    .h(px(148.))
                                    .px(px(32.))
                                    .py(px(20.))
                                    .child(dot_grid(p.rail))
                                    .child(Textarea::new(&self.try_field).appearance(false).h_full().text_size(px(20.)).text_color(p.fg)),
                            )
                            .child(dashes(false, p.rail)),
                    )
                    .child(
                        div()
                            .h(px(18.))
                            .text_size(px(13.))
                            .children(hint)
                            .when(ready && self.heard, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .text_color(p.primary)
                                        .child(icon("check", 14., p.primary))
                                        .child("That's it. It works anywhere you type."),
                                )
                            }),
                    ),
            )
    }

    fn done(&self, p: &Palette, cx: &App) -> Div {
        let pick = self.pick();
        let speech_note = match self.speech_job(cx) {
            Some(job @ Job::Running(..)) => format!("Downloading · {}%", (job.fraction() * 100.) as u32),
            Some(Job::Failed(_)) => "Didn't download".to_string(),
            _ if self.speech_ready() => "Ready".to_string(),
            _ => "Not downloaded".to_string(),
        };
        let (where_, what) = if self.api {
            ("With your API key", PROVIDERS[self.provider].0.to_string())
        } else {
            let what = match self.cleanup_job(cx) {
                Some(job @ Job::Running(..)) => format!("{} · {}%", job_label(&job, self.local_model), (job.fraction() * 100.) as u32),
                Some(Job::Failed(_)) => "Ollama didn't get set up".to_string(),
                _ => self.local_model.to_string(),
            };
            ("On this computer", what)
        };
        let keys = match self.platform {
            Platform::Mac => ("Option Shift Space", "Hold or tap"),
            Platform::Windows => ("Ctrl Shift Space", "Hold or tap"),
            Platform::Linux => ("F9", "Super Ctrl X toggles"),
        };
        let cells = [
            ("Speech", format!("{}{}", pick.name, if self.english { " (English)" } else { "" }), speech_note),
            ("Cleanup", where_.to_string(), what),
            ("Keys", keys.0.to_string(), keys.1.to_string()),
        ]
        .into_iter()
        .map(|(label, value, note)| {
            div()
                .flex()
                .flex_col()
                .w_full()
                .gap(px(6.))
                .px(px(24.))
                .py(px(20.))
                .child(div().text_size(px(12.)).text_color(p.dim).child(label))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).whitespace_nowrap().child(value))
                .child(div().text_size(px(13.)).text_color(p.dim).whitespace_nowrap().child(note))
                .into_any_element()
        })
        .collect();
        let body = match self.platform {
            Platform::Linux => "Nothing runs until you press your keys. Open Localflow from your launcher to change any of this.",
            Platform::Mac => "Localflow starts when you sign in and stays out of the way. Open it again to change any of this.",
            Platform::Windows => "Localflow starts when you sign in and stays out of the way. Open it from the Start menu to change any of this.",
        };
        let (primary, primary_fg) = (p.primary, p.primary_fg);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(18.))
                    .px(px(40.))
                    .pt(px(36.))
                    .pb(px(28.))
                    .child(div().size(px(48.)).flex().items_center().justify_center().with_spring(
                        "done-check",
                        SpringAnimation::new(SPRING).to(1.).from(0.4),
                        move |el, v| {
                            let s = 48. * v.max(0.);
                            el.child(
                                div()
                                    .size(px(s))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(primary)
                                    .opacity(v.clamp(0., 1.))
                                    .child(icon("check", 20. * v.clamp(0., 1.2), primary_fg)),
                            )
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .child(div().text_size(px(28.)).line_height(px(34.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child("You're set"))
                            .child(div().max_w(px(448.)).text_center().text_size(px(15.)).line_height(px(24.)).text_color(p.dim).child(body)),
                    ),
            )
            .child(band(cells, p))
            .child(
                div()
                    .px(px(40.))
                    .pt(px(22.))
                    .text_center()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(p.dim)
                    .child("Talk the way you think. Say “wait, actually…” to change your mind, “new paragraph” to break it up, and “…and send it” in a coding agent."),
            )
    }

    fn header(&self, p: &Palette) -> Div {
        let steps = self.platform.steps();
        let step = self.step();
        div()
            .relative()
            .flex_none()
            .h(px(48.))
            .flex()
            .items_center()
            .justify_center()
            .when(step != Step::Welcome, |d| {
                d.child(div().flex().items_center().gap(px(20.)).children(steps.iter().enumerate().filter(|(_, s)| !matches!(s, Step::Welcome | Step::Done)).map(
                    |(i, s)| {
                        let (dot, text) = if i <= self.index { (p.primary, if i == self.index { p.fg } else { p.fg.opacity(0.55) }) } else { (p.rail, p.faint) };
                        div().flex().items_center().gap(px(6.)).text_size(px(12.)).text_color(text).child(div().size(px(6.)).rounded_full().bg(dot)).child(s.label())
                    },
                )))
            })
    }

    fn footer(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let step = self.step();
        let status: Option<Div> = if let Some(e) = &self.error {
            Some(div().text_color(p.record).child(e.clone()))
        } else if step == Step::Done {
            None
        } else {
            let cleanup = if step == Step::Cleanup { None } else { self.cleanup_job(cx) };
            match self.speech_job(cx) {
                Some(Job::Done) | None if matches!(cleanup, Some(Job::Running(..))) => cleanup.map(|job| job_progress(&job, self.local_model, p)),
                None if matches!(cleanup, Some(Job::Failed(_))) => Some(div().text_color(p.record).child("Ollama didn't get set up: go back to Cleanup to try again")),
                Some(job @ Job::Running(..)) => Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(progress_bar(job.fraction(), p))
                        .child(format!("Downloading {}", self.pick().name))
                        .child(mono(format!("{}%", (job.fraction() * 100.) as u32), p)),
                ),
                Some(Job::Failed(e)) => Some(div().text_color(p.record).child(format!("The speech model didn't download: {e}"))),
                Some(Job::Done) => Some(div().flex().items_center().gap(px(8.)).child(icon("check", 14., p.primary)).child("Speech model ready")),
                None => None,
            }
        };
        let label = match step {
            Step::Done => "Start dictating",
            Step::Permissions if !(self.accessibility && self.mic == Mic::Allowed) => "Skip for now",
            _ => "Continue",
        };
        let enabled = self.can_continue(cx);
        div()
            .flex_none()
            .flex()
            .flex_col()
            .child(dashes(false, p.rail))
            .child(
                div()
                    .h(px(64.))
                    .px(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.)).text_color(p.dim).children(status))
                    .child(ghost_button("back", "Back", p).on_click(cx.listener(|this, _, _, cx| {
                        if this.index > 0 {
                            this.go(this.index - 1, cx)
                        }
                    })))
                    .child(primary_button("next", label, enabled, p).when(enabled, |b| b.on_click(cx.listener(|this, _, window, cx| this.next(window, cx))))),
            )
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(window);
        if self.focus_try {
            self.focus_try = false;
            self.try_field.update(cx, |field, cx| field.focus(window, cx));
        }
        let step = self.step();
        let middle: Vec<Step> = self.platform.steps().iter().copied().filter(|s| !matches!(s, Step::Welcome | Step::Done)).collect();
        let progress = match step {
            Step::Welcome => 0.,
            Step::Done => 1.,
            s => (middle.iter().position(|m| *m == s).unwrap_or(0) as f32 + 0.5) / middle.len() as f32,
        };
        let content = match step {
            Step::Welcome => self.welcome(&p, cx),
            Step::Speech => self.speech(&p, window, cx),
            Step::Cleanup => self.cleanup(&p, window, cx),
            Step::Permissions => self.permissions(&p, cx),
            Step::Keys => self.keys(&p, cx),
            Step::Try => self.try_it(&p, cx),
            Step::Done => self.done(&p, cx),
        };
        let direction = self.direction;
        let rail = p.rail;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .font_family(look::SANS)
            .text_size(px(14.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" && !event.is_held && this.step() != Step::Try {
                    this.next(window, cx);
                }
            }))
            .child(self.header(&p))
            .child(
                div().relative().flex_none().child(dashes(false, rail)).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h(px(1.))
                        .bg(p.primary)
                        .with_spring("progress", spring(SOFT, progress), |el, v| el.w(relative(v.clamp(0., 1.)))),
                ),
            )
            .child(
                div().flex_1().min_h_0().flex().justify_center().overflow_hidden().child(
                    div()
                        .relative()
                        .w(px(COLUMN))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(div().absolute().top_0().bottom_0().left_0().child(dashes(true, rail)))
                        .child(div().absolute().top_0().bottom_0().right_0().child(dashes(true, rail)))
                        .child(
                            div()
                                .relative()
                                .size_full()
                                .flex()
                                .flex_col()
                                .child(content)
                                .with_animation(
                                    SharedString::from(format!("step-{}", self.index)),
                                    Animation::new(Duration::from_millis(450)).with_easing(ease_out_quint()),
                                    move |el, t| el.opacity(t.clamp(0., 1.)).left(px((1. - t) * 28. * direction)),
                                ),
                        ),
                ),
            )
            .when(step != Step::Welcome, |d| d.child(self.footer(&p, cx)))
    }
}
