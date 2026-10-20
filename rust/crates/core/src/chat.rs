//! One chat request to the model: Ollama's native API, the Anthropic Messages
//! API, or any OpenAI-compatible chat API.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use crate::prompts::HAIKU;
use crate::{Error, Localflow, Result, err};

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub role: &'static str,
    pub content: String,
}

impl Msg {
    pub fn system(content: impl Into<String>) -> Msg {
        Msg { role: "system", content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Msg {
        Msg { role: "user", content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Msg {
        Msg { role: "assistant", content: content.into() }
    }
    fn json(&self) -> Value {
        json!({"role": self.role, "content": self.content})
    }
}

pub(crate) fn messages_json(messages: &[Msg]) -> Value {
    Value::Array(messages.iter().map(Msg::json).collect())
}

enum PostError {
    Status(u16, String),
    Other(String),
}

impl From<PostError> for Error {
    fn from(e: PostError) -> Error {
        match e {
            PostError::Status(code, detail) => Error(format!("HTTP {code}: {detail}")),
            PostError::Other(msg) => Error(msg),
        }
    }
}

impl Localflow {
    pub fn chat(&self, model: &str, messages: &[Msg], num_predict: usize, format: Option<&Value>) -> Result<String> {
        if model.is_empty() {
            return err("no model: set LOCALFLOW_MODEL to the provider's model name");
        }
        if self.cfg.anthropic {
            self.anthropic_chat(messages, num_predict)
        } else if !self.cfg.api_url.is_empty() {
            self.api_chat(model, messages, num_predict)
        } else {
            self.ollama_chat(model, messages, num_predict, format)
        }
    }

    fn api_key(&self) -> Result<String> {
        if let Some(key) = &self.cfg.api_key {
            return Ok(key.clone());
        }
        let path = self.cfg.key_file();
        std::fs::read_to_string(&path).map(|k| k.trim().to_string()).map_err(|_| {
            Error(format!("no API key: set LOCALFLOW_API_KEY or put it in {}", path.display()))
        })
    }

    fn post(&self, url: &str, headers: &[(&str, &str)], body: &Value) -> std::result::Result<Value, PostError> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        match req.send_json(body) {
            Ok(resp) => resp.into_json().map_err(|e| PostError::Other(format!("bad answer: {e}"))),
            Err(ureq::Error::Status(code, resp)) => {
                let detail: String = resp.into_string().unwrap_or_default().chars().take(300).collect();
                Err(PostError::Status(code, detail))
            }
            Err(e) => Err(PostError::Other(e.to_string())),
        }
    }

    fn api_chat(&self, model: &str, messages: &[Msg], num_predict: usize) -> Result<String> {
        let minimal = self.api_minimal.load(Ordering::Relaxed);
        let mut body = json!({"model": model, "messages": messages_json(messages)});
        if !minimal {
            body["temperature"] = json!(0);
            body["max_tokens"] = json!(num_predict);
        }
        let auth = format!("Bearer {}", self.api_key()?);
        let answer = match self.post(&format!("{}/chat/completions", self.cfg.api_url), &[("Authorization", &auth)], &body)
        {
            Ok(answer) => answer,
            Err(PostError::Status(400, _)) if !minimal => {
                self.api_minimal.store(true, Ordering::Relaxed);
                return self.api_chat(model, messages, num_predict).inspect_err(|_| {
                    self.api_minimal.store(false, Ordering::Relaxed);
                });
            }
            Err(e) => return Err(e.into()),
        };
        let choice = &answer["choices"][0];
        if choice["finish_reason"] == "length" {
            return err("model output was cut off (a thinking model? pick a non-reasoning one)");
        }
        Ok(choice["message"]["content"].as_str().unwrap_or_default().to_string())
    }

    fn anthropic_chat(&self, messages: &[Msg], num_predict: usize) -> Result<String> {
        let system: Vec<&str> = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect();
        let mut turns: Vec<Value> = messages.iter().filter(|m| m.role != "system").map(Msg::json).collect();
        if turns.len() > 1 {
            // Rules and examples are the same on every dictation: cache them, so
            // only the new transcript is processed from scratch.
            let i = turns.len() - 2;
            let text = turns[i]["content"].take();
            turns[i]["content"] = json!([{"type": "text", "text": text, "cache_control": {"type": "ephemeral"}}]);
        }
        // Only Haiku 5.5, whatever model was asked for; thinking off, it only slows cleanup down.
        let mut body = json!({"model": HAIKU, "max_tokens": num_predict, "messages": turns,
                              "thinking": {"type": "disabled"}});
        if !system.is_empty() {
            body["system"] = json!(system.join("\n\n"));
        }
        let key = self.api_key()?;
        let headers = [("x-api-key", key.as_str()), ("anthropic-version", "2023-06-01")];
        let answer = self.post(&format!("{}/messages", self.cfg.api_url), &headers, &body)?;
        match answer["stop_reason"].as_str() {
            Some("max_tokens") => return err("model output was cut off"),
            Some("refusal") => return err("model declined"),
            _ => {}
        }
        let blocks = answer["content"].as_array().map(Vec::as_slice).unwrap_or_default();
        Ok(blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
    }

    /// Load-time options (use_mmap) must match between warm-up and real
    /// requests, or Ollama reloads the model and the warm-up is wasted.
    pub(crate) fn ollama_options(&self, num_predict: usize) -> Value {
        json!({"temperature": 0, "num_predict": num_predict, "use_mmap": self.cfg.use_mmap})
    }

    fn ollama_chat(&self, model: &str, messages: &[Msg], num_predict: usize, format: Option<&Value>) -> Result<String> {
        let mut body = json!({
            "model": model,
            "messages": messages_json(messages),
            "stream": false,
            "think": false,
            "keep_alive": self.cfg.keep_alive,
            "options": self.ollama_options(num_predict),
        });
        if let Some(format) = format {
            body["format"] = format.clone();
        }
        let answer = self.post(&format!("{}/api/chat", self.cfg.ollama), &[], &body)?;
        if answer["done_reason"] == "length" {
            // Hit num_predict: the text is cut off, so don't use it.
            return err("model output was cut off");
        }
        Ok(answer["message"]["content"].as_str().unwrap_or_default().to_string())
    }

    /// Load the model and read the rules and examples so they're ready when the
    /// dictation arrives (nothing to load on a hosted API).
    ///
    /// qwen3.5 has recurrent layers, so Ollama can't reuse a cached prompt
    /// prefix by trimming it; it can only resume from a checkpoint saved at the
    /// end of an earlier prompt. Every dictation shares the same ~2,800-token
    /// start (rules + examples), so sending exactly that start saves a
    /// checkpoint the dictation then resumes from: ~1.1 s less per dictation on an M4.
    pub fn warm(&self, model: &str) -> Result<()> {
        if !self.cfg.api_url.is_empty() {
            return Ok(());
        }
        let body = json!({
            "model": model,
            "messages": messages_json(&crate::prefix_messages(&self.rules(), self.cfg.asides)),
            "stream": false,
            "think": false,
            "keep_alive": self.cfg.keep_alive,
            "options": self.ollama_options(1),
        });
        self.post(&format!("{}/api/chat", self.cfg.ollama), &[], &body)?;
        Ok(())
    }
}
