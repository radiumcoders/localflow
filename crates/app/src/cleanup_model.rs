//! localflow's cleanup model on this computer: downloaded like a speech model
//! (checked by size and SHA-256, resumable), then run by localflow itself, so
//! there's nothing else to install.

use std::sync::atomic::AtomicBool;

use localflow_core::Config;
use localflow_core::local::MODEL;

/// Get the model `cfg` names: download ours unless it's here; a .gguf file
/// of your own has to exist. `progress` gets (bytes done, bytes in all).
pub fn get_ready(cfg: &Config, progress: &mut dyn FnMut(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
    let path = cfg.model_file();
    if cfg.model != MODEL.id {
        return if path.exists() { Ok(()) } else { Err(format!("{} doesn't exist", path.display())) };
    }
    localflow_speech::models::download_file(MODEL.name, MODEL.url, MODEL.size, MODEL.sha256, &path, progress, cancel)
        .map_err(|e| e.0)
}

/// "localflow's cleanup model (351 MB)": for buttons and notes.
pub fn described() -> String {
    format!("{} ({} MB)", MODEL.name, MODEL.size_mb())
}
