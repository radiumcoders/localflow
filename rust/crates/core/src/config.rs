//! Settings: LOCALFLOW_* variables, also read from the env file install.py
//! writes (~/.config/localflow/env, %APPDATA%\localflow\env, one VAR=value per
//! line). A variable set in the environment wins.
//!
//! ```text
//! LOCALFLOW_MODEL         model name (default: qwen3.5:4b; ignored with
//!                         LOCALFLOW_API_URL=anthropic, which always uses claude-haiku-5-5)
//! LOCALFLOW_API_URL       use this API instead of Ollama: openai, openrouter, groq, gemini,
//!                         anthropic, mistral, deepseek, or any OpenAI-compatible base URL
//! LOCALFLOW_API_KEY       API key (default: read from the api_key file next to env)
//! LOCALFLOW_REVIEW_MODEL  model for the repair pass (default: same; "off" to skip)
//! LOCALFLOW_OLLAMA        Ollama base URL (default: http://127.0.0.1:11434)
//! LOCALFLOW_KEEP_ALIVE    how long the model stays loaded after use (default: 1h, 2m on macOS)
//! LOCALFLOW_USE_MMAP      1 to also keep the model file mapped in RAM (default: off, on on macOS)
//! LOCALFLOW_SPEECH_MODEL  Handy speech model id (default: the one selected in Handy)
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::prompts::{HAIKU, PROVIDERS};

#[derive(Debug, Clone)]
pub struct Config {
    /// A hosted API's base URL, or empty for Ollama.
    pub api_url: String,
    pub anthropic: bool,
    /// Empty when a hosted API is set without a model (chat() says so).
    pub model: String,
    /// The repair pass's model, or "off".
    pub review_model: String,
    pub ollama: String,
    pub keep_alive: String,
    /// Load the model into GPU/CPU buffers instead of also keeping the file
    /// mapped in RAM: halves RAM use (2.1 -> 1.1 GB on GPU) at the same speed.
    /// Apple Silicon is the opposite: CPU and GPU share memory, so the GPU
    /// reads the mapped file directly (1.3 instead of 4.8 GB for qwen3.5:4b).
    pub use_mmap: bool,
    pub api_key: Option<String>,
    /// Handy's speech model, or None for the one selected in Handy's settings.
    pub speech_model: Option<String>,
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
        let model = if anthropic {
            HAIKU.to_string()
        } else {
            var("LOCALFLOW_MODEL")
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| if api_url.is_empty() { "qwen3.5:4b".into() } else { String::new() })
        };
        let mut review_model = var("LOCALFLOW_REVIEW_MODEL").unwrap_or_else(|| model.clone());
        if anthropic && review_model != "off" {
            review_model = HAIKU.to_string();
        }
        let mac = cfg!(target_os = "macos");
        Config {
            ollama: var("LOCALFLOW_OLLAMA")
                .unwrap_or_else(|| "http://127.0.0.1:11434".into())
                .trim_end_matches('/')
                .to_string(),
            keep_alive: var("LOCALFLOW_KEEP_ALIVE").unwrap_or_else(|| (if mac { "2m" } else { "1h" }).into()),
            use_mmap: var("LOCALFLOW_USE_MMAP").unwrap_or_else(|| (if mac { "1" } else { "0" }).into()) == "1",
            api_key: var("LOCALFLOW_API_KEY").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
            speech_model: var("LOCALFLOW_SPEECH_MODEL").filter(|m| !m.is_empty()),
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

    /// Where cleanup runs, for messages: the API's URL or Ollama's.
    pub fn backend(&self) -> &str {
        if self.api_url.is_empty() { &self.ollama } else { &self.api_url }
    }
}
