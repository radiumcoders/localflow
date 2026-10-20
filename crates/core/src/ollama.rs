//! Ollama itself, for setup: which models it has, and pulling one.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Error, Localflow, Result, err};

impl Localflow {
    /// The models Ollama has, or an error when it isn't running.
    pub fn ollama_models(&self) -> Result<Vec<String>> {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(3)).build();
        let tags: Value = agent
            .get(&format!("{}/api/tags", self.cfg.ollama))
            .call()
            .map_err(|e| Error(format!("Ollama isn't running at {} ({e})", self.cfg.ollama)))?
            .into_json()
            .map_err(|e| Error(e.to_string()))?;
        Ok(tags["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str()).map(String::from).collect())
    }

    /// Download `model` into Ollama. `progress` gets (bytes done, bytes in
    /// all) as layers arrive; `cancel` stops it between updates.
    pub fn ollama_pull(&self, model: &str, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<()> {
        let response = self
            .agent
            .post(&format!("{}/api/pull", self.cfg.ollama))
            .send_json(json!({"model": model, "stream": true}))
            .map_err(|e| Error(format!("couldn't pull {model}: {e}")))?;
        // One JSON object per line; each layer reports its own total.
        let mut layers: Vec<(String, u64, u64)> = vec![];
        for line in BufReader::new(response.into_reader()).lines() {
            if cancel.load(Ordering::Relaxed) {
                return err("cancelled");
            }
            let line = line.map_err(|e| Error(e.to_string()))?;
            let Ok(update) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(e) = update["error"].as_str() {
                return err(format!("couldn't pull {model}: {e}"));
            }
            if let (Some(digest), Some(total)) = (update["digest"].as_str(), update["total"].as_u64()) {
                let done = update["completed"].as_u64().unwrap_or(0);
                match layers.iter_mut().find(|(d, _, _)| d == digest) {
                    Some(layer) => (layer.1, layer.2) = (done, total),
                    None => layers.push((digest.to_string(), done, total)),
                }
                progress(layers.iter().map(|l| l.1).sum(), layers.iter().map(|l| l.2).sum());
            }
            if update["status"] == "success" {
                return Ok(());
            }
        }
        err(format!("couldn't pull {model}: Ollama stopped answering"))
    }
}
