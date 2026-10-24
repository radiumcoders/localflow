//! Settings: LOCALFLOW_* variables, also read from the env file install.py
//! writes (~/.config/localflow/env, %APPDATA%\localflow\env, one VAR=value per
//! line). A variable set in the environment wins.
//!
//! ```text
//! LOCALFLOW_MODEL         the cleanup model: on this computer, localflow-clean (the default) or
//!                         a .gguf file; with an API, the provider's model name (ignored with
//!                         LOCALFLOW_API_URL=anthropic, which always uses claude-haiku-5-5)
//! LOCALFLOW_API_URL       use this API instead of this computer: openai, openrouter, groq,
//!                         gemini, anthropic, mistral, deepseek, or any OpenAI-compatible base URL
//! LOCALFLOW_API_KEY       API key (default: read from the api_key file next to env)
//! LOCALFLOW_REVIEW_MODEL  an API's model for the repair pass (default: same; "off" to skip;
//!                         always off on this computer)
//! LOCALFLOW_CLEANUP_GPU   off to clean up on the CPU (default: the GPU when there is one)
//! LOCALFLOW_SPEECH_MODEL  speech model id (see `localflow models`; default: the recommended one)
//! LOCALFLOW_SPEECH_GPU    off to transcribe on the CPU (default: the GPU when there is one)
//! LOCALFLOW_ASIDES        keep to paste what you say to yourself or others while dictating
//!                         (default: drop it, see prompts::ASIDES_RULE)
//! LOCALFLOW_SUBMIT        in a coding agent, press Enter after pasting: said (when you end
//!                         with "send it", the default), always, or never
//! LOCALFLOW_HISTORY       off to keep no history of what you dictated (default: the last 200,
//!                         for `localflow history`)
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::agent::Submit;
use crate::local::{MODEL, models_dir};
use crate::prompts::{HAIKU, PROVIDERS};

#[derive(Debug, Clone)]
pub struct Config {
    /// A hosted API's base URL, or empty for cleanup on this computer.
    pub api_url: String,
    pub anthropic: bool,
    /// On this computer: localflow-clean or a .gguf file. Empty when a hosted
    /// API is set without a model (chat() says so).
    pub model: String,
    /// The repair pass's model, or "off".
    pub review_model: String,
    /// Run the cleanup model on the GPU when there is one.
    pub cleanup_gpu: bool,
    pub api_key: Option<String>,
    /// The speech model's id, or None for the recommended one.
    pub speech_model: Option<String>,
    /// Transcribe on the GPU when there is one.
    pub speech_gpu: bool,
    /// Leave out asides: what you say to yourself or to someone else, and
    /// what you mutter (rule 12).
    pub asides: bool,
    /// When a prompt dictated into a coding agent is sent.
    pub submit: Submit,
    /// Keep what you said and what went out, for `localflow history`.
    pub history: bool,
    /// Where the env file, the api_key file and your learned words.txt live.
    pub config_dir: PathBuf,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn home() -> PathBuf {
    PathBuf::from(env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

pub fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        PathBuf::from(env("APPDATA").unwrap_or_default())
    } else {
        env("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
    };
    base.join("localflow")
}

fn off(value: Option<String>) -> bool {
    matches!(value.map(|v| v.trim().to_lowercase()).as_deref(), Some("off" | "0" | "no" | "false"))
}

fn read_env_file(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(k, _)| k.starts_with("LOCALFLOW_"))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

impl Config {
    pub fn from_env() -> Config {
        Config::from_vars(&env, config_dir())
    }

    /// `get` reads a variable (the environment, or a test's); the env file in
    /// `dir` fills in the ones it doesn't have.
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>, dir: PathBuf) -> Config {
        let file = read_env_file(&dir.join("env"));
        let var = |k: &str| get(k).or_else(|| file.get(k).cloned());
        let provider = var("LOCALFLOW_API_URL").unwrap_or_default().trim().to_string();
        let api_url = PROVIDERS
            .iter()
            .find(|(name, _)| *name == provider.to_lowercase())
            .map_or(provider.as_str(), |(_, url)| url)
            .trim_end_matches('/')
            .to_string();
        let anthropic = api_url.starts_with("https://api.anthropic.com");
        let model = var("LOCALFLOW_MODEL").map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        let model = if anthropic {
            HAIKU.to_string()
        } else if api_url.is_empty() {
            // An Ollama model name from before localflow ran its own model means ours.
            model.filter(|m| m.ends_with(".gguf")).unwrap_or_else(|| MODEL.id.into())
        } else {
            model.unwrap_or_default()
        };
        let mut review_model = var("LOCALFLOW_REVIEW_MODEL").unwrap_or_else(|| model.clone());
        if anthropic && review_model != "off" {
            review_model = HAIKU.to_string();
        }
        if api_url.is_empty() {
            // localflow-clean was trained for cleanup only: there's no repair pass for it to do.
            review_model = "off".into();
        }
        Config {
            cleanup_gpu: !off(var("LOCALFLOW_CLEANUP_GPU")),
            api_key: var("LOCALFLOW_API_KEY").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
            speech_model: var("LOCALFLOW_SPEECH_MODEL").filter(|m| !m.is_empty()),
            speech_gpu: !off(var("LOCALFLOW_SPEECH_GPU")),
            asides: !matches!(var("LOCALFLOW_ASIDES").map(|v| v.trim().to_lowercase()).as_deref(), Some("keep" | "off" | "0" | "no")),
            submit: Submit::parse(var("LOCALFLOW_SUBMIT").as_deref()),
            history: !off(var("LOCALFLOW_HISTORY")),
            api_url,
            anthropic,
            model,
            review_model,
            config_dir: dir,
        }
    }

    /// The key lives in a file only you can read, not in a service definition.
    pub fn key_file(&self) -> PathBuf {
        self.config_dir.join("api_key")
    }

    /// Your own vocabulary: learning from your fixes adds to it.
    pub fn user_words(&self) -> PathBuf {
        self.config_dir.join("words.txt")
    }

    /// Cleanup on this computer, not with an API.
    pub fn local(&self) -> bool {
        self.api_url.is_empty()
    }

    /// The local model's file: ours in the models folder, or the .gguf named.
    pub fn model_file(&self) -> PathBuf {
        if self.model == MODEL.id { models_dir().join(MODEL.file) } else { PathBuf::from(&self.model) }
    }

    /// Where cleanup runs, for messages: the API's URL, or this computer.
    pub fn backend(&self) -> &str {
        if self.local() { "this computer" } else { &self.api_url }
    }
}

/// Change settings in the env file in `dir`: each (VAR, Some(value)) is set,
/// each (VAR, None) removed. Other lines, comments included, stay as they are.
pub fn save_settings(dir: &Path, changes: &[(&str, Option<&str>)]) -> std::io::Result<()> {
    let path = dir.join("env");
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|line| !changes.iter().any(|(k, _)| line.trim().split_once('=').is_some_and(|(name, _)| name == *k)))
        .map(String::from)
        .collect();
    lines.extend(changes.iter().filter_map(|(k, v)| v.map(|v| format!("{k}={v}"))));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("env.tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    std::fs::rename(&tmp, &path)
}

/// Save the API key where Config reads it, readable only by you. An empty
/// key removes the file.
pub fn save_api_key(dir: &Path, key: &str) -> std::io::Result<()> {
    let path = dir.join("api_key");
    if key.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&path)?, format!("{}\n", key.trim()).as_bytes())
}
