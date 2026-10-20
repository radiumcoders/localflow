//! The settings window: the speech model (downloaded here), where cleanup
//! runs (Ollama on this computer, or a hosted model with your key), asides
//! and learning, and the keys to dictate with. Saves to the env file the
//! rest of localflow reads (`localflow_core::Config`).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::select::{SearchableVec, Select, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme, Disableable, IndexPath, Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use localflow_core::{Config, Localflow, Msg, prompts::HAIKU, save_api_key, save_settings};
use localflow_speech::models;

/// Hosted providers, as shown, and their LOCALFLOW_API_URL names. The last
/// one takes any OpenAI-compatible base URL.
const PROVIDERS: &[(&str, &str)] = &[
    ("Anthropic (Claude Haiku)", "anthropic"),
    ("OpenAI", "openai"),
    ("OpenRouter", "openrouter"),
    ("Groq", "groq"),
    ("Google Gemini", "gemini"),
    ("Mistral", "mistral"),
    ("DeepSeek", "deepseek"),
    ("Other (OpenAI-compatible URL)", ""),
];
const LEARN: &[(&str, &str)] = &[("On its own", "auto"), ("Only with the learn key", "key"), ("Off", "off")];
const SUBMIT: &[(&str, &str)] = &[("When you say \"send it\"", "said"), ("Always", "always"), ("Never", "never")];
const LANGUAGES: &[(&str, &str)] = &[
    ("Detect it", ""),
    ("English", "en"),
    ("German", "de"),
    ("French", "fr"),
    ("Spanish", "es"),
    ("Italian", "it"),
    ("Portuguese", "pt"),
    ("Dutch", "nl"),
    ("Polish", "pl"),
    ("Hindi", "hi"),
    ("Japanese", "ja"),
    ("Chinese", "zh"),
];
/// Local cleanup models offered first (any Ollama model works).
const LOCAL_MODELS: &[&str] = &["qwen3.5:4b", "qwen3.5:2b"];

type Choice = Entity<SelectState<SearchableVec<&'static str>>>;

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Idle,
    Running(Option<(u64, u64)>),
    Done(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Ollama {
    Checking,
    Down(String),
    Up(Vec<String>),
}

struct State {
    quit_with_settings: bool,
    window: Option<AnyWindowHandle>,
}

impl Global for State {}

pub fn init(quit_with_settings: bool, cx: &mut App) {
    cx.set_global(State { quit_with_settings, window: None });
    cx.on_window_closed(|cx, id| {
        let state = cx.global_mut::<State>();
        if state.window.is_some_and(|w| w.window_id() == id) {
            state.window = None;
            if state.quit_with_settings {
                cx.quit();
            }
        }
    })
    .detach();
}

/// Open the settings window, or bring it to the front.
pub fn open(cx: &mut App) {
    if let Some(window) = cx.global::<State>().window
        && window.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(640.), px(780.)), cx);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions { title: Some("localflow".into()), ..Default::default() }),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(480.), px(420.))),
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
    language: Choice,
    gpu: bool,
    api: bool,
    provider: Choice,
    api_model: Entity<InputState>,
    api_url: Entity<InputState>,
    api_key: Entity<InputState>,
    has_key: bool,
    local_model: Entity<InputState>,
    ollama: Ollama,
    pull: Job,
    check: Job,
    asides: bool,
    learn: Choice,
    submit: Choice,
    history: bool,
    saved: Job,
}

fn index_of<T: PartialEq>(items: &[(&'static str, T)], value: &T) -> usize {
    items.iter().position(|(_, v)| v == value).unwrap_or(0)
}

fn choice(items: &[(&'static str, &'static str)], selected: usize, window: &mut Window, cx: &mut App) -> Choice {
    let labels: Vec<&'static str> = items.iter().map(|(label, _)| *label).collect();
    cx.new(|cx| SelectState::new(SearchableVec::new(labels), Some(IndexPath::new(selected)), window, cx))
}

/// The value behind a select's label.
fn chosen(state: &Choice, items: &[(&'static str, &'static str)], cx: &App) -> &'static str {
    let label = state.read(cx).selected_value().copied();
    items.iter().find(|(l, _)| Some(*l) == label).map_or(items[0].1, |(_, v)| *v)
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let cfg = Config::from_env();
        let file = |k: &str| env_var(k).or_else(|| env_file_value(&cfg, k));
        let provider_name = file("LOCALFLOW_API_URL").unwrap_or_default().trim().to_lowercase();
        let custom = !provider_name.is_empty() && !PROVIDERS.iter().any(|(_, id)| *id == provider_name);
        let provider_ix = if custom { PROVIDERS.len() - 1 } else { index_of(PROVIDERS, &provider_name.as_str()) };
        let language = cfg.language.clone().unwrap_or_default();
        let learn = file("LOCALFLOW_LEARN").unwrap_or_else(|| "auto".into()).trim().to_lowercase();
        let submit = cfg.submit.value().unwrap_or("said");
        let has_key = cfg.api_key.is_some() || cfg.key_file().exists();
        let api_model = if cfg.anthropic || cfg.api_url.is_empty() { String::new() } else { cfg.model.clone() };
        let local_model = if cfg.api_url.is_empty() { cfg.model.clone() } else { LOCAL_MODELS[0].to_string() };

        let view = SettingsView {
            speech_model: models::resolve(cfg.speech_model.as_deref()).id,
            downloads: vec![],
            language: choice(LANGUAGES, index_of(LANGUAGES, &language.as_str()), window, cx),
            gpu: cfg.speech_gpu,
            api: !cfg.api_url.is_empty(),
            provider: choice(PROVIDERS, provider_ix, window, cx),
            api_model: cx.new(|cx| InputState::new(window, cx).placeholder("the provider's model name").default_value(api_model)),
            api_url: cx.new(|cx| {
                let url = if custom { cfg.api_url.clone() } else { String::new() };
                InputState::new(window, cx).placeholder("https://my-server.example/v1").default_value(url)
            }),
            api_key: cx.new(|cx| {
                let hint = if has_key { "saved (type to replace)" } else { "your API key" };
                InputState::new(window, cx).placeholder(hint).masked(true)
            }),
            has_key,
            local_model: cx.new(|cx| InputState::new(window, cx).default_value(local_model)),
            ollama: Ollama::Checking,
            pull: Job::Idle,
            check: Job::Idle,
            asides: cfg.asides,
            learn: choice(LEARN, index_of(LEARN, &learn.as_str()), window, cx),
            submit: choice(SUBMIT, index_of(SUBMIT, &submit), window, cx),
            history: cfg.history,
            saved: Job::Idle,
            cfg,
        };
        for entity in [&view.provider, &view.language, &view.learn, &view.submit] {
            cx.observe(entity, |_, _, cx| cx.notify()).detach();
        }
        view.check_ollama(cx);
        view
    }

    fn provider_id(&self, cx: &App) -> &'static str {
        chosen(&self.provider, PROVIDERS, cx)
    }

    /// The settings as chosen in the window, as env-file changes.
    fn changes(&self, cx: &App) -> Vec<(&'static str, Option<String>)> {
        let mut changes: Vec<(&'static str, Option<String>)> = vec![];
        if self.api {
            let id = self.provider_id(cx);
            let url = if id.is_empty() { self.api_url.read(cx).value().trim().to_string() } else { id.to_string() };
            let model = if id == "anthropic" { HAIKU.to_string() } else { self.api_model.read(cx).value().trim().to_string() };
            changes.push(("LOCALFLOW_API_URL", Some(url)));
            changes.push(("LOCALFLOW_MODEL", Some(model).filter(|m| !m.is_empty())));
        } else {
            changes.push(("LOCALFLOW_API_URL", None));
            let model = self.local_model.read(cx).value().trim().to_string();
            changes.push(("LOCALFLOW_MODEL", Some(model).filter(|m| !m.is_empty())));
        }
        changes.push(("LOCALFLOW_SPEECH_MODEL", Some(self.speech_model.to_string())));
        changes.push(("LOCALFLOW_SPEECH_GPU", (!self.gpu).then(|| "off".into())));
        let language = chosen(&self.language, LANGUAGES, cx);
        changes.push(("LOCALFLOW_LANGUAGE", (!language.is_empty()).then(|| language.into())));
        changes.push(("LOCALFLOW_ASIDES", (!self.asides).then(|| "keep".into())));
        let learn = chosen(&self.learn, LEARN, cx);
        changes.push(("LOCALFLOW_LEARN", (learn != "auto").then(|| learn.into())));
        let submit = chosen(&self.submit, SUBMIT, cx);
        changes.push(("LOCALFLOW_SUBMIT", (submit != "said").then(|| submit.into())));
        changes.push(("LOCALFLOW_HISTORY", (!self.history).then(|| "off".into())));
        changes
    }

    /// The cleanup engine as set up in the window (saved or not), for "Test".
    fn engine(&self, cx: &App) -> Localflow {
        let changes = self.changes(cx);
        let key = self.api_key.read(cx).value().trim().to_string();
        let dir = self.cfg.config_dir.clone();
        let get = move |k: &str| -> Option<String> {
            if k == "LOCALFLOW_API_KEY" && !key.is_empty() {
                return Some(key.clone());
            }
            match changes.iter().find(|(name, _)| *name == k) {
                Some((_, value)) => Some(value.clone().unwrap_or_default()),
                None => env_var(k),
            }
        };
        Localflow::new(Config::from_vars(&get, dir))
    }

    fn check_ollama(&self, cx: &mut Context<Self>) {
        let lf = Localflow::new(self.cfg.clone());
        let task = cx.background_executor().spawn(async move { lf.ollama_models() });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ollama = match found {
                    Ok(models) => Ollama::Up(models),
                    Err(e) => Ollama::Down(e.0),
                };
                cx.notify();
            });
        })
        .detach();
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

    fn pull(&mut self, cx: &mut Context<Self>) {
        let model = self.local_model.read(cx).value().trim().to_string();
        let lf = Localflow::new(self.cfg.clone());
        let cancel = AtomicBool::new(false);
        self.job(
            cx,
            move |progress| lf.ollama_pull(&model, progress, &cancel).map_err(|e| e.0),
            |this, job| this.pull = job,
            |_| "Downloaded".into(),
        );
        self.check_ollama(cx);
    }

    fn test(&mut self, cx: &mut Context<Self>) {
        let lf = self.engine(cx);
        self.job(
            cx,
            move |_| {
                let model = lf.cfg.model.clone();
                lf.chat(&model, &[Msg::user("Reply with just: ok")], 32, None)
                    .map(|_| format!("Works: {model} via {}", lf.cfg.backend()))
                    .map_err(|e| e.0)
            },
            |this, job| this.check = job,
            |ok| ok,
        );
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let changes = self.changes(cx);
        let dir = self.cfg.config_dir.clone();
        let key = self.api_key.read(cx).value().trim().to_string();
        let pairs: Vec<(&str, Option<&str>)> = changes.iter().map(|(k, v)| (*k, v.as_deref())).collect();
        let result = save_settings(&dir, &pairs).and_then(|_| if key.is_empty() { Ok(()) } else { save_api_key(&dir, &key) });
        self.saved = match result.map_err(|e| e.to_string()).and_then(|_| after_save()) {
            Ok(()) => {
                self.has_key |= !key.is_empty();
                Job::Done("Saved".into())
            }
            Err(e) => Job::Failed(format!("Couldn't save: {e}")),
        };
        cx.notify();
    }
}

/// Once saved: the app where it runs from, when it isn't there yet (the
/// Linux AppImage copies itself out, a Mac app adds its login item).
fn after_save() -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?.display().to_string();
    let linux = cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some();
    let mac = cfg!(target_os = "macos") && me.contains(".app/Contents/MacOS/") && !me.starts_with("/Volumes/");
    if linux || mac {
        crate::setup::install_app(false)?;
    }
    Ok(())
}

/// A value from the env file only (not the environment), for settings
/// Config doesn't keep in its own form.
fn env_file_value(cfg: &Config, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(cfg.config_dir.join("env")).ok()?;
    text.lines().filter_map(|l| l.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1e6)
}

fn percent((done, total): (u64, u64)) -> f32 {
    if total == 0 { 0. } else { done as f32 * 100. / total as f32 }
}

impl SettingsView {
    fn section(&self, title: &str, body: impl IntoElement, cx: &App) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().font_semibold().text_color(cx.theme().muted_foreground).child(title.to_uppercase()))
            .child(body)
    }

    fn card(cx: &App) -> Div {
        div()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
    }

    fn row(label: impl Into<SharedString>, hint: Option<SharedString>, control: impl IntoElement, cx: &App) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px_4()
            .py_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().child(label.into()))
                    .children(hint.map(|h| div().text_xs().text_color(cx.theme().muted_foreground).child(h))),
            )
            .child(div().flex_none().child(control))
    }

    fn status(job: &Job, cx: &App) -> Option<Div> {
        let theme = cx.theme();
        match job {
            Job::Idle => None,
            Job::Running(Some(p)) if p.1 > 0 => Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(160.)).child(Progress::new("progress").value(percent(*p))))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{} of {}", mb(p.0), mb(p.1)))),
            ),
            Job::Running(_) => Some(div().text_xs().text_color(theme.muted_foreground).child("Working…")),
            Job::Done(text) => Some(div().text_xs().text_color(theme.success).child(text.clone())),
            Job::Failed(text) => Some(div().text_xs().text_color(theme.danger).child(text.clone())),
        }
    }

    fn speech(&self, cx: &mut Context<Self>) -> Div {
        let mut list = Self::card(cx);
        for (i, model) in models::MODELS.iter().enumerate() {
            let id = model.id;
            let selected = self.speech_model == id;
            let job = self.downloads.iter().find(|(d, _, _)| *d == id).map(|(_, j, _)| j.clone());
            let installed = models::installed(id);
            let running = matches!(job, Some(Job::Running(_)));
            let action = if installed {
                div().text_xs().text_color(cx.theme().success).child("Installed").into_any_element()
            } else if running {
                div().into_any_element()
            } else {
                Button::new(SharedString::from(format!("get-{id}")))
                    .label("Download")
                    .small()
                    .outline()
                    .on_click(cx.listener(move |this, _, _, cx| this.download(id, cx)))
                    .into_any_element()
            };
            let theme = cx.theme();
            list = list.child(
                div()
                    .id(SharedString::from(format!("model-{id}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .when(i > 0, |d| d.border_t_1().border_color(theme.border))
                    .when(selected, |d| d.bg(theme.accent))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.speech_model = id;
                        if !models::installed(id) && !this.downloads.iter().any(|(d, j, _)| *d == id && matches!(j, Job::Running(_))) {
                            this.download(id, cx);
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded_full()
                            .border_1()
                            .border_color(if selected { theme.primary } else { theme.border })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |d| d.child(div().size(px(8.)).rounded_full().bg(theme.primary))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .items_baseline()
                                    .child(div().text_sm().font_medium().child(model.name))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{} MB", model.size_mb())))
                                    .when(model.id == models::default_model().id, |d| {
                                        d.child(div().text_xs().font_semibold().text_color(theme.success).child("Recommended"))
                                    }),
                            )
                            .child(div().text_xs().text_color(theme.muted_foreground).child(model.note))
                            .children(job.as_ref().and_then(|j| Self::status(j, cx))),
                    )
                    .child(action),
            );
        }
        let options = Self::card(cx)
            .child(Self::row("Language", Some("What you speak".into()), Select::new(&self.language).w(px(180.)).small(), cx))
            .child(div().border_t_1().border_color(cx.theme().border).child(Self::row(
                "Use the GPU",
                Some("Faster when there is one; off uses only the CPU".into()),
                Switch::new("gpu").checked(self.gpu).on_click(cx.listener(|this, checked, _, cx| {
                    this.gpu = *checked;
                    cx.notify();
                })),
                cx,
            )));
        div().flex().flex_col().gap_3().child(list).child(options)
    }

    fn cleanup(&self, cx: &mut Context<Self>) -> Div {
        let tabs = TabBar::new("cleanup-mode")
            .segmented()
            .selected_index(self.api as usize)
            .children([Tab::new().label("On this computer"), Tab::new().label("With your API key")])
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                this.api = *index == 1;
                this.check = Job::Idle;
                cx.notify();
            }));
        let body = if self.api { self.api_settings(cx) } else { self.local_settings(cx) };
        let test = div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("test")
                    .label("Test")
                    .small()
                    .outline()
                    .loading(matches!(self.check, Job::Running(_)))
                    .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
            )
            .children(Self::status(&self.check, cx));
        div().flex().flex_col().gap_3().child(tabs).child(Self::card(cx).child(body).child(test))
    }

    fn local_settings(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let installed: &[String] = match &self.ollama {
            Ollama::Up(models) => models,
            _ => &[],
        };
        let status = match &self.ollama {
            Ollama::Checking => div().text_xs().text_color(theme.muted_foreground).child("Looking for Ollama…"),
            Ollama::Up(models) => div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("Ollama is running, with {} model{}", models.len(), if models.len() == 1 { "" } else { "s" })),
            Ollama::Down(_) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(theme.danger).child("Ollama isn't running."))
                .child(Button::new("get-ollama").label("Get Ollama").small().link().on_click(|_, _, cx| cx.open_url("https://ollama.com/download")))
                .child(Button::new("recheck").label("Check again").small().ghost().on_click(cx.listener(|this, _, _, cx| {
                    this.ollama = Ollama::Checking;
                    this.check_ollama(cx);
                    cx.notify();
                }))),
        };
        let typed = self.local_model.read(cx).value().trim().to_string();
        let have = installed.iter().any(|m| *m == typed || m.strip_suffix(":latest") == Some(typed.as_str()));
        let mut picks: Vec<String> = LOCAL_MODELS.iter().map(|m| m.to_string()).collect();
        for m in installed {
            if !picks.contains(m) {
                picks.push(m.clone());
            }
        }
        let chips = div().flex().flex_wrap().gap_1().children(picks.into_iter().enumerate().map(|(i, m)| {
            let input = self.local_model.clone();
            Button::new(("pick", i)).label(m.clone()).xsmall().ghost().on_click(move |_, window, cx| {
                input.update(cx, |state, cx| state.set_value(m.clone(), window, cx));
            })
        }));
        let pull = (!have && matches!(self.ollama, Ollama::Up(_))).then(|| {
            Button::new("pull")
                .label("Download")
                .small()
                .outline()
                .disabled(matches!(self.pull, Job::Running(_)))
                .on_click(cx.listener(|this, _, _, cx| this.pull(cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .child(div().text_sm().child("Cleanup model (Ollama)"))
            .child(div().flex().gap_2().child(Input::new(&self.local_model).small().flex_1()).children(pull))
            .child(chips)
            .children(Self::status(&self.pull, cx))
            .child(status)
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                "Nothing leaves your computer. qwen3.5:4b needs ~3.3 GB of disk and a GPU for fast long dictation; 2b is faster.",
            ))
    }

    fn api_settings(&self, cx: &mut Context<Self>) -> Div {
        let id = self.provider_id(cx);
        let theme = cx.theme();
        let field = |label: &str, input: AnyElement| {
            div().flex().flex_col().gap_1().child(div().text_sm().child(label.to_string())).child(input)
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .px_4()
            .py_3()
            .child(field("Provider", Select::new(&self.provider).small().into_any_element()))
            .when(id.is_empty(), |d| d.child(field("Base URL", Input::new(&self.api_url).small().into_any_element())))
            .child(if id == "anthropic" {
                field(
                    "Model",
                    div().text_sm().text_color(theme.muted_foreground).child("Claude Haiku 5.5 (always, on Anthropic)").into_any_element(),
                )
            } else {
                field("Model", Input::new(&self.api_model).small().into_any_element())
            })
            .child(field("API key", Input::new(&self.api_key).small().mask_toggle().into_any_element()))
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "Every dictation is sent to this provider. The key is saved where only you can read it.",
            ))
    }

    fn dictation(&self, cx: &mut Context<Self>) -> Div {
        Self::card(cx)
            .child(Self::row(
                "Leave out asides",
                Some("What you say to yourself or to someone else, and what you mutter, isn't pasted".into()),
                Switch::new("asides").checked(self.asides).on_click(cx.listener(|this, checked, _, cx| {
                    this.asides = *checked;
                    cx.notify();
                })),
                cx,
            ))
            .child(div().border_t_1().border_color(cx.theme().border).child(Self::row(
                "Learn from your fixes",
                Some("When you correct a misheard word, it's spelled right from then on".into()),
                Select::new(&self.learn).w(px(200.)).small(),
                cx,
            )))
            .child(div().border_t_1().border_color(cx.theme().border).child(Self::row(
                "Send prompts to coding agents",
                Some("Press Enter after pasting into Claude Code, Codex...".into()),
                Select::new(&self.submit).w(px(200.)).small(),
                cx,
            )))
            .child(div().border_t_1().border_color(cx.theme().border).child(Self::row(
                "Keep a history",
                Some("What you said and what went out, on this computer only".into()),
                Switch::new("history").checked(self.history).on_click(cx.listener(|this, checked, _, cx| {
                    this.history = *checked;
                    cx.notify();
                })),
                cx,
            )))
    }

    fn keys(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let text = |t: &str| div().text_sm().child(t.to_string());
        let body = if cfg!(target_os = "macos") {
            div().flex().flex_col().gap_1().px_4().py_3()
                .child(text("Hold Option+Shift+Space and talk, or tap it, talk, and tap again. Esc cancels."))
                .child(text("Option+Shift+L learns from the sentence you fixed (select it first)."))
        } else if cfg!(windows) {
            div().flex().flex_col().gap_1().px_4().py_3()
                .child(text("Hold Ctrl+Shift+Space and talk, or tap it, talk, and tap again. Esc cancels."))
                .child(text("Ctrl+Alt+Space learns from the sentence you fixed (select it first)."))
        } else {
            let bindings = crate::session::linux_bindings();
            let copy = bindings.clone();
            div()
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .child(text("Bind keys to localflow's commands. On Hyprland, add these to ~/.config/hypr/bindings.lua:"))
                .child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(theme.muted)
                        .font_family("monospace")
                        .text_xs()
                        .children(bindings.lines().map(|l| div().child(l.to_string()))),
                )
                .child(div().flex().child(Button::new("copy-bindings").label("Copy").small().outline().on_click(
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())),
                )))
        };
        Self::card(cx).child(body)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let ready = models::installed(self.speech_model);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_2xl().font_bold().child("localflow"))
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Talk, and get clean text where you type.")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(div().size(px(8.)).rounded_full().bg(if ready { theme.success } else { theme.warning }))
                    .child(if ready { "Speech model ready" } else { "Download a speech model" }),
            );
        let footer = div()
            .flex()
            .items_center()
            .justify_end()
            .gap_3()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.background)
            .children(Self::status(&self.saved, cx))
            .child(Button::new("save").label("Save").primary().on_click(cx.listener(|this, _, _, cx| this.save(cx))));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.secondary)
            .text_color(theme.foreground)
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .px_6()
                            .py_5()
                            .child(header)
                            .child(self.section("Speech to text", self.speech(cx), cx))
                            .child(self.section("Cleanup", self.cleanup(cx), cx))
                            .child(self.section("While you dictate", self.dictation(cx), cx))
                            .child(self.section("Keys", self.keys(cx), cx)),
                    ),
            )
            .child(footer)
    }
}
