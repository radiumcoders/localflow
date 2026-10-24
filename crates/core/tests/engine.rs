//! The cleanup's settings, learning and env file: no model or network needed.

use std::path::PathBuf;

use localflow_core::{Config, Localflow, NO_FILES, corrections, edited_region, save_settings};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("localflow-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(vars: &[(&str, &str)], dir: PathBuf) -> Config {
    let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Config::from_vars(&move |k| vars.iter().find(|(name, _)| name == k).map(|(_, v)| v.clone()), dir)
}

#[test]
fn local_default() {
    let cfg = config(&[], temp_dir("local"));
    assert_eq!(cfg.model, "localflow-clean");
    assert!(cfg.cleanup_gpu);
    assert!(cfg.model_file().ends_with("localflow/models/localflow-clean-q4_k_m.gguf"));
    // An Ollama model or a hosted API saved before localflow ran its own model means ours;
    // a .gguf file is used as it is.
    let cfg = config(&[("LOCALFLOW_MODEL", "qwen3.5:4b")], temp_dir("local-old"));
    assert_eq!(cfg.model, "localflow-clean");
    let cfg = config(&[("LOCALFLOW_API_URL", "anthropic"), ("LOCALFLOW_MODEL", "claude-haiku-5-5")], temp_dir("local-api"));
    assert_eq!(cfg.model, "localflow-clean");
    let cfg = config(&[("LOCALFLOW_MODEL", "/m/mine.gguf"), ("LOCALFLOW_CLEANUP_GPU", "off")], temp_dir("local-file"));
    assert_eq!((cfg.model_file(), cfg.cleanup_gpu), (PathBuf::from("/m/mine.gguf"), false));
}

#[test]
fn local_model_not_downloaded_says_so() {
    let lf = Localflow::new(config(&[("LOCALFLOW_MODEL", "/nowhere/missing.gguf")], temp_dir("local-missing")));
    assert!(!lf.cfg.local_model_installed());
    let e = lf.clean("um hi", NO_FILES).unwrap_err();
    assert!(e.0.contains("isn't downloaded"), "{e}");
}

#[test]
fn env_file() {
    let dir = temp_dir("envfile");
    std::fs::write(dir.join("env"), "LOCALFLOW_MODEL=/m/mine.gguf\nNOT_OURS=1\n").unwrap();
    assert_eq!(config(&[], dir.clone()).model, "/m/mine.gguf");
    assert_eq!(config(&[("LOCALFLOW_MODEL", "/m/other.gguf")], dir).model, "/m/other.gguf"); // the environment wins
}

#[test]
fn learn_corrections() {
    let lf = Localflow::new(config(&[], temp_dir("learn")));
    let pair = |h: &str, m: &str| (h.to_string(), m.to_string());
    assert_eq!(corrections("Push it to git hub and ask Claw about local flow.",
                           "Push it to GitHub and ask Claude about localflow."),
               Some(vec![pair("git hub", "GitHub"), pair("Claw", "Claude"), pair("local flow", "localflow")]));
    assert_eq!(corrections("I think there car is red", "I think their car is red"), Some(vec![])); // grammar
    assert_eq!(corrections("Push it to git hub", "Something else entirely here"), None);
    assert_eq!(lf.learn(&[pair("git hub", "GitHub"), pair("slack", "Slack")]).unwrap(),
               vec![pair("git hub", "GitHub"), pair("slack", "Slack")]);
    assert_eq!(lf.learn(&[pair("git hub", "GitHub")]).unwrap(), vec![]); // already known
    assert_eq!(lf.learn(&[pair("get hub", "GitHub")]).unwrap(), vec![pair("get hub", "GitHub")]);
    let words = std::fs::read_to_string(lf.cfg.user_words()).unwrap();
    assert!(words.contains(r#"GitHub: often heard as "git hub", "get hub""#), "{words}");
    assert!(lf.misheard_hints().contains(&("GitHub".into(), vec!["git hub".into(), "get hub".into()])));
    assert!(lf.vocabulary().contains(&"Slack".to_string()));
    assert_eq!(edited_region("Ask Claw to push it to git hub.", "Hi. Ask Claude to push it to GitHub. And more"),
               Some("Ask Claude to push it to GitHub.".into()));
}

#[test]
fn settings_are_saved_to_the_env_file() {
    let dir = temp_dir("save");
    std::fs::write(dir.join("env"), "# mine\nLOCALFLOW_MODEL=qwen3.5:2b\nLOCALFLOW_CLEANUP_GPU=off\n").unwrap();
    save_settings(&dir, &[("LOCALFLOW_MODEL", None), ("LOCALFLOW_SPEECH_MODEL", Some("base.en"))]).unwrap();
    let text = std::fs::read_to_string(dir.join("env")).unwrap();
    assert_eq!(text, "# mine\nLOCALFLOW_CLEANUP_GPU=off\nLOCALFLOW_SPEECH_MODEL=base.en\n");
    let cfg = config(&[], dir);
    assert!(!cfg.cleanup_gpu);
    assert_eq!(cfg.speech_model.as_deref(), Some("base.en"));
}
