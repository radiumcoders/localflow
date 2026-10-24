//! Settings: LOCALFLOW_* variables, also read from the env file install.py
//! writes (~/.config/localflow/env, %APPDATA%\localflow\env, one VAR=value per
//! line). A variable set in the environment wins.
//!
//! ```text
//! LOCALFLOW_MODEL         the cleanup model: localflow-clean (the default) or a .gguf file
//! LOCALFLOW_CLEANUP_GPU   off to clean up on the CPU (default: the GPU when there is one)
//! LOCALFLOW_SPEECH_MODEL  speech model id (see `localflow models`; default: the recommended one)
//! LOCALFLOW_SPEECH_GPU    off to transcribe on the CPU (default: the GPU when there is one)
//! LOCALFLOW_SUBMIT        in a coding agent, press Enter after pasting: said (when you end
//!                         with "send it", the default), always, or never
//! LOCALFLOW_HISTORY       off to keep no history of what you dictated (default: the last 200,
//!                         for `localflow history`)
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::agent::Submit;
use crate::local::{MODEL, models_dir};

#[derive(Debug, Clone)]
pub struct Config {
    /// localflow-clean or a .gguf file.
    pub model: String,
    /// Run the cleanup model on the GPU when there is one.
    pub cleanup_gpu: bool,
    /// The speech model's id, or None for the recommended one.
    pub speech_model: Option<String>,
    /// Transcribe on the GPU when there is one.
    pub speech_gpu: bool,
    /// When a prompt dictated into a coding agent is sent.
    pub submit: Submit,
    /// Keep what you said and what went out, for `localflow history`.
    pub history: bool,
    /// Where the env file and your learned words.txt live.
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
        // An Ollama model name from before localflow ran its own model means ours.
        let model = var("LOCALFLOW_MODEL").map(|m| m.trim().to_string()).filter(|m| m.ends_with(".gguf"));
        Config {
            cleanup_gpu: !off(var("LOCALFLOW_CLEANUP_GPU")),
            speech_model: var("LOCALFLOW_SPEECH_MODEL").filter(|m| !m.is_empty()),
            speech_gpu: !off(var("LOCALFLOW_SPEECH_GPU")),
            submit: Submit::parse(var("LOCALFLOW_SUBMIT").as_deref()),
            history: !off(var("LOCALFLOW_HISTORY")),
            model: model.unwrap_or_else(|| MODEL.id.into()),
            config_dir: dir,
        }
    }

    /// Your own vocabulary: learning from your fixes adds to it.
    pub fn user_words(&self) -> PathBuf {
        self.config_dir.join("words.txt")
    }

    /// The local model's file: ours in the models folder, or the .gguf named.
    pub fn model_file(&self) -> PathBuf {
        if self.model == MODEL.id { models_dir().join(MODEL.file) } else { PathBuf::from(&self.model) }
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
