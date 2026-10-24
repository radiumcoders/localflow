//! Cleanup on this computer: localflow's own model, localflow-clean (Qwen3.5
//! 0.8B fine-tuned on the rules, English vocabulary only), run in this process
//! by localflow-llm. It learned the rules from examples, so it gets the
//! one-line instruction it was trained with instead of them, and no examples.
//! The format must match ~/Projects/localflow-model's fmt.py.

use std::path::PathBuf;

use crate::cleanup::user_turn;
use crate::{Config, Error, Localflow, Msg, Result, err};

/// The model file localflow downloads, like a speech model.
pub struct ModelFile {
    /// What LOCALFLOW_MODEL names.
    pub id: &'static str,
    pub name: &'static str,
    /// The file name, in the repository and on disk.
    pub file: &'static str,
    pub url: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    pub sha256: &'static str,
}

impl ModelFile {
    pub fn size_mb(&self) -> u64 {
        self.size.div_ceil(1_000_000)
    }
}

pub const MODEL: ModelFile = ModelFile {
    id: "localflow-clean",
    name: "localflow's cleanup model",
    file: "localflow-clean-q4_k_m.gguf",
    url: "https://huggingface.co/Radiumcoders/localflow-clean/resolve/main/localflow-clean-q4_k_m.gguf",
    size: 350_373_280,
    sha256: "0fa7ba4e4a38b50067a2dbd6eb01ce2e0aa6db755afc4a216cffd27566eb3206",
};

const SYSTEM: &str = "Clean up the dictation.";

/// Room for prompt and answer, in tokens: a few minutes of speech, the
/// vocabulary and a project's file list fit.
const CONTEXT: u32 = 4096;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Where models live: ~/.local/share/localflow/models,
/// ~/Library/Application Support/localflow/models,
/// %LOCALAPPDATA%\localflow\models (the speech models' folder).
pub fn models_dir() -> PathBuf {
    let home = || PathBuf::from(env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default());
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Local"))
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else {
        env("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
    };
    base.join("localflow").join("models")
}

/// Qwen3.5's chat format with thinking off: everything before the answer.
fn chat_prompt(messages: &[Msg]) -> String {
    let mut prompt: String =
        messages.iter().map(|m| format!("<|im_start|>{}\n{}<|im_end|>\n", m.role, m.content)).collect();
    prompt += "<|im_start|>assistant\n<think>\n\n</think>\n\n";
    prompt
}

/// The request the model was trained on: the instruction, the speaker's
/// vocabulary, then the dictation (and the project's files).
pub fn cleanup_prompt(vocabulary: &[String], text: &str, files: &[impl AsRef<str>]) -> String {
    let mut system = SYSTEM.to_string();
    if !vocabulary.is_empty() {
        system += "\n\nSpeaker's vocabulary (use these exact spellings):\n";
        system += &vocabulary.iter().map(|w| format!("- {w}")).collect::<Vec<_>>().join("\n");
    }
    chat_prompt(&[Msg::system(system), Msg::user(user_turn(text, files))])
}

impl Config {
    /// Downloaded and complete (or a .gguf file you named that exists).
    pub fn local_model_installed(&self) -> bool {
        let path = self.model_file();
        if self.model == MODEL.id { std::fs::metadata(&path).is_ok_and(|m| m.len() == MODEL.size) } else { path.exists() }
    }
}

impl Localflow {
    /// Load the model unless it's loaded. It stays loaded as long as this
    /// Localflow: one dictation.
    pub(crate) fn load_local(&self) -> Result<()> {
        let mut engine = self.engine.lock().unwrap();
        if engine.is_none() {
            if !self.cfg.local_model_installed() {
                return err(format!("{} isn't downloaded yet", MODEL.name));
            }
            *engine = Some(localflow_llm::Model::load(&self.cfg.model_file(), self.cfg.cleanup_gpu, CONTEXT).map_err(Error)?);
        }
        Ok(())
    }

    /// The model's answer to a prompt in its own format.
    pub(crate) fn local_complete(&self, prompt: &str, max_tokens: usize) -> Result<String> {
        self.load_local()?;
        let mut engine = self.engine.lock().unwrap();
        let (text, cut_off) = engine.as_mut().unwrap().complete(prompt, max_tokens).map_err(Error)?;
        if cut_off {
            return err("model output was cut off");
        }
        Ok(text)
    }

    pub(crate) fn local_chat(&self, messages: &[Msg], num_predict: usize) -> Result<String> {
        self.local_complete(&chat_prompt(messages), num_predict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_the_training_format() {
        let prompt = cleanup_prompt(&["Claude".into(), "Pine Script: heard as \"Py script\"".into()], "um hi", &["src/a.rs"]);
        assert_eq!(
            prompt,
            "<|im_start|>system\nClean up the dictation.\n\nSpeaker's vocabulary (use these exact spellings):\n\
             - Claude\n- Pine Script: heard as \"Py script\"<|im_end|>\n<|im_start|>user\n<dictation>um hi</dictation>\n\
             <files>\nsrc/a.rs\n</files><|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
        let none: &[&str] = &[];
        assert!(cleanup_prompt(&[], "hi", none).starts_with("<|im_start|>system\nClean up the dictation.<|im_end|>\n"));
    }
}
