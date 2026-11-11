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
fn local_model_missing_says_so() {
    // A .gguf file of your own that isn't there: not found (it isn't a download).
    let lf = Localflow::new(config(&[("LOCALFLOW_MODEL", "/nowhere/missing.gguf")], temp_dir("local-missing")));
    assert!(!lf.cfg.local_model_installed());
    let e = lf.clean("um hi", NO_FILES).unwrap_err();
    assert_eq!(e.0, "the cleanup model's file wasn't found: /nowhere/missing.gguf");
    // localflow's own model, not downloaded.
    let lf = Localflow::new(config(&[], temp_dir("local-ours")));
    if !lf.cfg.local_model_installed() {
        assert_eq!(lf.check_local().unwrap_err().0, "localflow's cleanup model isn't downloaded yet");
    }
}

#[test]
fn check_asks_claude_when_haiku_cleans_up() {
    // `localflow --check` with Claude Haiku chosen: Claude is asked (here
    // there's no key, and it says so) instead of the local model answering "ok".
    let lf = Localflow::new(config(&[("LOCALFLOW_CLEANUP", "claude-haiku-5-5")], temp_dir("check-haiku")));
    assert!(lf.cfg.cleanup_haiku && lf.cfg.anthropic_key.is_none());
    assert_eq!(lf.check_haiku().unwrap_err().0, "no Anthropic API key (Settings > Cleanup)");
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

#[test]
fn microphone() {
    assert_eq!(config(&[], temp_dir("mic")).mic, None); // the system's default
    assert_eq!(config(&[("LOCALFLOW_MIC", " ")], temp_dir("mic-empty")).mic, None);
    // A name with spaces and a colon, as the settings window saves it.
    let dir = temp_dir("mic-file");
    save_settings(&dir, &[("LOCALFLOW_MIC", Some("Jabra Evolve2 65: USB Audio"))]).unwrap();
    assert_eq!(config(&[], dir.clone()).mic.as_deref(), Some("Jabra Evolve2 65: USB Audio"));
    assert_eq!(config(&[("LOCALFLOW_MIC", "")], dir).mic, None); // the environment wins, even empty
}

#[test]
fn diagram_model_setting() {
    let cfg = config(&[], temp_dir("diagram"));
    assert_eq!(cfg.diagram_model, "qwen3.5-2b");
    assert!(cfg.diagrams());
    assert!(cfg.diagram_model_file().ends_with("localflow/models/Qwen3.5-2B-Q4_K_M.gguf"));
    for (value, expected) in [("qwen3.5-4b", "qwen3.5-4b"), ("off", "off"), ("OFF", "off"), ("/m/draw.gguf", "/m/draw.gguf"),
                              ("gpt-9", "qwen3.5-2b"), ("", "qwen3.5-2b")] {
        let cfg = config(&[("LOCALFLOW_DIAGRAM_MODEL", value)], temp_dir("diagram-set"));
        assert_eq!(cfg.diagram_model, expected, "{value}");
    }
    let off = config(&[("LOCALFLOW_DIAGRAM_MODEL", "off")], temp_dir("diagram-off"));
    assert!(!off.diagrams() && !off.diagram_model_installed());
    // Off, a file that isn't there, a model not downloaded: drawing says which
    // (instead of failing to load, or asking for a download that wouldn't help).
    let request = localflow_core::diagram::Request { kind: None, visualize: true, about: "wake up then work".into() };
    let why = |cfg: Config| Localflow::new(cfg).diagram(&request).unwrap_err().0;
    assert_eq!(why(off), "diagrams are off (Settings > Visualize, or LOCALFLOW_DIAGRAM_MODEL)");
    let missing = config(&[("LOCALFLOW_DIAGRAM_MODEL", "/nowhere/draw.gguf")], temp_dir("diagram-missing"));
    assert_eq!(why(missing), "the diagram model's file wasn't found: /nowhere/draw.gguf");
    if !cfg.diagram_model_installed() {
        assert_eq!(why(cfg), "the diagram model isn't downloaded yet");
    }
}

#[test]
fn haiku_diagrams_need_a_key() {
    let dir = temp_dir("haiku");
    let cfg = config(&[("LOCALFLOW_DIAGRAM_MODEL", "claude-haiku-5-5")], dir.clone());
    assert!(cfg.haiku_diagrams() && cfg.anthropic_key.is_none() && !cfg.diagram_model_installed());
    // Without a key, Haiku says so (and there's no local model here to draw instead).
    let request = localflow_core::diagram::Request { kind: None, visualize: true, about: "wake up then work".into() };
    let lf = Localflow::new(cfg);
    if lf.cfg.local_diagram_model().is_none() {
        assert!(lf.diagram(&request).unwrap_err().0.contains("API key"));
    }
    // The key: LOCALFLOW_ANTHROPIC_API_KEY, else ANTHROPIC_API_KEY, else the saved file.
    localflow_core::save_anthropic_key(&dir, " sk-ant-file \n").unwrap();
    let key = |vars: &[(&str, &str)]| config(vars, dir.clone()).anthropic_key.map(|k| k.0);
    assert_eq!(key(&[]), Some("sk-ant-file".into()));
    assert_eq!(key(&[("ANTHROPIC_API_KEY", "sk-ant-env")]), Some("sk-ant-env".into()));
    assert_eq!(key(&[("ANTHROPIC_API_KEY", "sk-ant-env"), ("LOCALFLOW_ANTHROPIC_API_KEY", "sk-ant-ours")]), Some("sk-ant-ours".into()));
    let cfg = config(&[("LOCALFLOW_DIAGRAM_MODEL", "claude-haiku-5-5")], dir.clone());
    assert!(cfg.diagram_model_installed());
    assert!(!format!("{cfg:?}").contains("sk-ant"), "the key shows in debug output");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("anthropic_api_key")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    localflow_core::save_anthropic_key(&dir, "").unwrap();
    assert_eq!(key(&[]), None);
}
