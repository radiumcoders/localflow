//! The cleanup against a fake API server, like tests/test_localflow.py: no key,
//! model or network needed. The fake "cleans up" by capitalizing the
//! dictation and adding a period.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use localflow_core::{Config, Localflow, Msg, NO_FILES, corrections, edited_region, prefix_messages, save_api_key,
                     save_settings};
use regex::Regex;
use serde_json::{Value, json};

/// (path, lowercased headers, body) of each request the fake API got.
type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>, Value)>>>;

fn fake_api() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
            let mut headers = vec![];
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                match line.trim_end().split_once(':') {
                    Some((k, v)) => headers.push((k.to_lowercase(), v.trim().to_string())),
                    None => break,
                }
            }
            let len: usize = headers.iter().find(|(k, _)| k == "content-length").map_or(0, |(_, v)| v.parse().unwrap());
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            let (code, answer) = reply(&path, &headers, &body);
            log.lock().unwrap().push((path, headers, body));
            let data = answer.to_string();
            let _ = write!(stream, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                                   Connection: close\r\n\r\n{data}", data.len());
        }
    });
    (url, seen)
}

fn reply(path: &str, headers: &[(String, String)], body: &Value) -> (u16, Value) {
    let last = &body["messages"].as_array().unwrap().last().unwrap()["content"];
    let last: String = match last {
        Value::Array(blocks) => blocks.iter().map(|b| b["text"].as_str().unwrap()).collect(),
        other => other.as_str().unwrap().to_string(),
    };
    let text = Regex::new(r"(?s)<dictation>(.*)</dictation>").unwrap()
        .captures(&last).map_or(last.clone(), |c| c[1].to_string());
    // Like the model with rule 12: what was muttered goes.
    let text = Regex::new(r"(?s)<quiet>.*?</quiet>").unwrap().replace_all(&text, " ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() && last.contains("<quiet>") {
        return (200, json!({"choices": [{"finish_reason": "stop", "message": {"content": "<aside>"}}]}));
    }
    let mut chars = text.chars();
    let mut answer = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    answer += chars.as_str().trim_end_matches('.');
    answer += ".";
    if last.contains("CHANGES:") {
        // Judging edits: learn the ones with a capital letter (names).
        let learn: Vec<u32> = Regex::new(r#"(?m)^(\d+)\. ".*" -> "(.*)"$"#).unwrap().captures_iter(&last)
            .filter(|c| c[2].starts_with(char::is_uppercase))
            .map(|c| c[1].parse().unwrap())
            .collect();
        answer = json!({"learn": learn}).to_string();
    }
    if path == "/v1/messages" {
        if !headers.iter().any(|(k, v)| k == "x-api-key" && v == "test-key") {
            return (401, json!({"type": "error", "error": {"type": "authentication_error"}}));
        }
        return (200, json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": answer}]}));
    }
    (200, json!({"choices": [{"finish_reason": "stop", "message": {"content": answer}}]}))
}

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

/// Anthropic, pointed at the fake server (which speaks its API).
fn anthropic(url: &str, extra: &[(&str, &str)], name: &str) -> Localflow {
    let mut vars = extra.to_vec(); // first, so they win
    vars.extend([("LOCALFLOW_API_URL", "anthropic"), ("LOCALFLOW_API_KEY", "test-key")]);
    let mut cfg = config(&vars, temp_dir(name));
    cfg.api_url = url.to_string();
    Localflow::new(cfg)
}

#[test]
fn local_default() {
    let cfg = config(&[], temp_dir("local"));
    assert_eq!((cfg.model.as_str(), cfg.review_model.as_str()), ("localflow-clean", "off"));
    assert!(cfg.local() && cfg.cleanup_gpu);
    assert!(cfg.model_file().ends_with("localflow/models/localflow-clean-q4_k_m.gguf"));
    // An Ollama model saved before localflow ran its own means ours; a .gguf file is used as it is.
    let cfg = config(&[("LOCALFLOW_MODEL", "qwen3.5:4b"), ("LOCALFLOW_REVIEW_MODEL", "qwen3.5:4b")], temp_dir("local-old"));
    assert_eq!((cfg.model.as_str(), cfg.review_model.as_str()), ("localflow-clean", "off"));
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
fn anthropic_is_always_haiku() {
    let (url, seen) = fake_api();
    let lf = anthropic(&url, &[("LOCALFLOW_MODEL", "claude-opus-5-5"), ("LOCALFLOW_REVIEW_MODEL", "claude-sonnet-5-5")],
                       "haiku");
    assert!(lf.cfg.anthropic);
    assert_eq!((lf.cfg.model.as_str(), lf.cfg.review_model.as_str()), ("claude-haiku-5-5", "claude-haiku-5-5"));
    assert_eq!(lf.clean("meet at 3 wait at 2", NO_FILES).unwrap(), "Meet at 3 wait at 2.");
    lf.chat("claude-opus-5-5", &[Msg::user("hi")], 8).unwrap();
    assert!(seen.lock().unwrap().iter().all(|(_, _, b)| b["model"] == "claude-haiku-5-5"));
}

#[test]
fn anthropic_request_shape() {
    let (url, seen) = fake_api();
    anthropic(&url, &[], "shape").clean("call mom", NO_FILES).unwrap();
    let seen = seen.lock().unwrap();
    let (path, headers, body) = &seen[0];
    assert_eq!(path, "/v1/messages");
    assert!(headers.contains(&("anthropic-version".into(), "2023-06-01".into())));
    assert_eq!(body["thinking"], json!({"type": "disabled"}));
    assert!(body.get("temperature").is_none());
    assert!(body["system"].as_str().unwrap().contains("dictation cleanup filter"));
    let messages = body["messages"].as_array().unwrap();
    assert!(messages.iter().all(|m| m["role"] == "user" || m["role"] == "assistant"));
    // Rules and examples are cached: the marker sits on the last example.
    assert_eq!(messages[messages.len() - 2]["content"][0]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn bad_key_keeps_the_transcript() {
    let (url, _) = fake_api();
    assert!(anthropic(&url, &[("LOCALFLOW_API_KEY", "wrong")], "badkey").clean("keep my words", NO_FILES).is_err());
}

#[test]
fn openai_compatible() {
    let (url, seen) = fake_api();
    let cfg = config(&[("LOCALFLOW_API_URL", &url), ("LOCALFLOW_MODEL", "small-model"), ("LOCALFLOW_API_KEY", "k")],
                     temp_dir("openai"));
    assert_eq!(Localflow::new(cfg).clean("call mom tomorrow", NO_FILES).unwrap(), "Call mom tomorrow.");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, "/v1/chat/completions");
    assert_eq!(seen[0].2["model"], "small-model");
    assert!(seen[0].1.contains(&("authorization".into(), "Bearer k".into())));
}

#[test]
fn provider_without_model_says_so() {
    let lf = Localflow::new(config(&[("LOCALFLOW_API_URL", "groq")], temp_dir("nomodel")));
    let e = lf.chat(&lf.cfg.model, &[Msg::user("hi")], 8).unwrap_err();
    assert!(e.0.contains("LOCALFLOW_MODEL"), "{e}");
}

#[test]
fn env_file() {
    let dir = temp_dir("envfile");
    std::fs::write(dir.join("env"), "LOCALFLOW_API_URL=anthropic\nNOT_OURS=1\n").unwrap();
    assert!(config(&[], dir.clone()).anthropic);
    assert!(!config(&[("LOCALFLOW_API_URL", "groq")], dir).anthropic); // the environment wins
}

#[test]
fn file_list_goes_with_the_dictation() {
    let (url, seen) = fake_api();
    let lf = anthropic(&url, &[], "files");
    lf.clean("look at the settings file", &["src/settings.py", "tests/"]).unwrap();
    lf.clean("call mom", NO_FILES).unwrap(); // no files: no list
    let seen = seen.lock().unwrap();
    let turn = |i: usize| seen[i].2["messages"].as_array().unwrap().last().unwrap()["content"].clone();
    assert!(turn(0).as_str().unwrap()
        .contains("<dictation>look at the settings file</dictation>\n<files>\nsrc/settings.py\ntests/\n</files>"));
    assert!(!turn(1).as_str().unwrap().contains("<files>"));
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
    assert!(lf.rules().contains("- Slack"));
}

#[test]
fn judges_which_fixes_are_mishearings() {
    let (url, seen) = fake_api();
    let lf = anthropic(&url, &[], "judge");
    let pasted = "Ask Claw to push it to git hub and ship it tuesday.";
    let sent = "Ask Claude to push it to GitHub and ship it wednesday.";
    let pairs = corrections(pasted, sent).unwrap();
    let keep = lf.mishearings(pasted, sent, &pairs, &lf.cfg.model).unwrap();
    // The fake learns capitalized fixes: names, not "wednesday".
    assert_eq!(keep, vec![("Claw".into(), "Claude".into()), ("git hub".into(), "GitHub".into())]);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(edited_region("Ask Claw to push it to git hub.", "Hi. Ask Claude to push it to GitHub. And more"),
               Some("Ask Claude to push it to GitHub.".into()));
}

#[test]
fn reasoning_models_get_a_minimal_request() {
    // A provider that rejects temperature/max_tokens (400) gets them dropped from then on.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
    let log = bodies.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut line, mut len) = (String::new(), 0);
            while reader.read_line(&mut line).unwrap() > 2 {
                if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                line.clear();
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            let (code, answer) = if body.get("temperature").is_some() {
                (400, json!({"error": "temperature not supported"}).to_string())
            } else {
                (200, json!({"choices": [{"finish_reason": "stop", "message": {"content": "Ok."}}]}).to_string())
            };
            log.lock().unwrap().push(body);
            let _ = write!(stream, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                           answer.len());
        }
    });
    let lf = Localflow::new(config(&[("LOCALFLOW_API_URL", &url), ("LOCALFLOW_MODEL", "gpt-5"),
                                     ("LOCALFLOW_API_KEY", "k")], temp_dir("minimal")));
    assert_eq!(lf.chat("gpt-5", &[Msg::user("hi")], 8).unwrap(), "Ok.");
    assert_eq!(lf.chat("gpt-5", &[Msg::user("hi")], 8).unwrap(), "Ok.");
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "one refusal, then minimal requests only");
    assert!(bodies[1].get("max_tokens").is_none() && bodies[2].get("max_tokens").is_none());
}

fn openai(url: &str, extra: &[(&str, &str)], name: &str) -> Localflow {
    let mut vars = extra.to_vec();
    vars.extend([("LOCALFLOW_API_URL", url), ("LOCALFLOW_API_KEY", "k"), ("LOCALFLOW_MODEL", "m")]);
    Localflow::new(config(&vars, temp_dir(name)))
}

#[test]
fn asides_are_left_out() {
    let (url, _) = fake_api();
    let lf = openai(&url, &[], "asides");
    assert!(lf.cfg.asides);
    assert!(lf.rules().contains("12. Asides"));
    assert_eq!(lf.clean("ship it <quiet>one sec, I'm on a call</quiet> today", NO_FILES).unwrap(), "Ship it today.");
    // Only an aside: nothing to paste (not the raw transcript).
    let r = lf.clean_detailed("<quiet>hang on, can you turn the music down</quiet>", NO_FILES).unwrap();
    assert_eq!((r.text.as_str(), r.verdict.as_str()), ("", "aside"));
    // The checks don't ask to put back what was muttered.
    assert!(localflow_core::find_problems("send it <quiet>maybe later</quiet> now", "Send it now.", &[]).is_empty());
}

#[test]
fn asides_can_be_kept() {
    let (url, _) = fake_api();
    let on = openai(&url, &[], "asides-on");
    let off = openai(&url, &[("LOCALFLOW_ASIDES", "keep")], "asides-off");
    assert!(!off.cfg.asides);
    assert!(!off.rules().contains("12. Asides"));
    let examples = localflow_core::prompts::ASIDE_EXAMPLES.len() * 2;
    assert_eq!(prefix_messages(&on.rules(), true).len(), prefix_messages(&off.rules(), false).len() + examples);
}

#[test]
fn settings_are_saved_to_the_env_file() {
    let dir = temp_dir("save");
    std::fs::write(dir.join("env"), "# mine\nLOCALFLOW_MODEL=qwen3.5:2b\nLOCALFLOW_CLEANUP_GPU=off\n").unwrap();
    save_settings(&dir, &[("LOCALFLOW_MODEL", None), ("LOCALFLOW_API_URL", Some("anthropic")),
                          ("LOCALFLOW_ASIDES", Some("keep"))]).unwrap();
    let text = std::fs::read_to_string(dir.join("env")).unwrap();
    assert_eq!(text, "# mine\nLOCALFLOW_CLEANUP_GPU=off\nLOCALFLOW_API_URL=anthropic\nLOCALFLOW_ASIDES=keep\n");
    save_api_key(&dir, " sk-test \n").unwrap();
    let cfg = config(&[], dir.clone());
    assert!(cfg.anthropic && !cfg.asides);
    assert!(!cfg.cleanup_gpu);
    assert_eq!(std::fs::read_to_string(cfg.key_file()).unwrap(), "sk-test\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(cfg.key_file()).unwrap().permissions().mode() & 0o777, 0o600);
    }
    save_api_key(&dir, "").unwrap();
    assert!(!cfg.key_file().exists());
}
