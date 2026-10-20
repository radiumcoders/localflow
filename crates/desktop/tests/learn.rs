//! Learning from your fixes, like tests/test_localflow.py: a fake API server
//! stands in for the cleanup model, temp folders for your home and editors.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use localflow_core::{Config, Localflow};
use localflow_desktop::learn::{
    SentMessages, Watch, last_dictation_at, learn_from, learned_message, remember_at, watch_with,
};
use regex::Regex;
use serde_json::{Value, json};

/// The fake judge learns the changes whose new words start with a capital (names).
/// Returns its URL and how many judging requests it got.
fn fake_api() -> (String, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let judged: Arc<Mutex<usize>> = Arc::default();
    let count = judged.clone();
    let change = Regex::new(r#"(?m)^(\d+)\. ".*" -> "(.*)"$"#).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut line, mut len) = (String::new(), 0);
            reader.read_line(&mut line).unwrap();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                match line.trim_end().split_once(':') {
                    Some((k, v)) if k.eq_ignore_ascii_case("content-length") => len = v.trim().parse().unwrap(),
                    Some(_) => {}
                    None => break,
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            let last = body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap_or_default().to_string();
            let mut answer = "Ok.".to_string();
            if last.contains("CHANGES:") {
                *count.lock().unwrap() += 1;
                let learn: Vec<u32> = change.captures_iter(&last)
                    .filter(|c| c[2].starts_with(char::is_uppercase))
                    .map(|c| c[1].parse().unwrap())
                    .collect();
                answer = json!({"learn": learn}).to_string();
            }
            let data = json!({"choices": [{"finish_reason": "stop", "message": {"content": answer}}]}).to_string();
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                                   Connection: close\r\n\r\n{data}", data.len());
        }
    });
    (url, judged)
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("localflow-learn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn localflow(config: PathBuf, vars: &[(&str, &str)]) -> Localflow {
    let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Localflow::new(Config::from_vars(&|k| vars.get(k).cloned(), config))
}

#[test]
fn learns_from_what_you_send() {
    // After a paste, the edited version you send to Claude Code is found in
    // its transcript; the fixes the model calls mishearings are learned.
    let (url, judged) = fake_api();
    let (home, config) = (temp("home"), temp("config"));
    let lf = localflow(config.clone(), &[("LOCALFLOW_API_URL", &url), ("LOCALFLOW_MODEL", "small"), ("LOCALFLOW_API_KEY", "k")]);
    let pasted = "Ask Claw to push it to git hub and ship it tuesday.";
    let last = home.join("last.json");
    remember_at(&last, pasted, 1.5).unwrap();
    let w = Watch {
        sent: SentMessages::watching(&home, vec![]),
        last_file: last.clone(),
        poll: Duration::from_millis(50),
        limit: Duration::from_secs(10),
        text_box: || None,
    };
    let writer = {
        let home = home.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let log = home.join(".claude/projects/-tmp-proj/session.jsonl");
            std::fs::create_dir_all(log.parent().unwrap()).unwrap();
            let sent = "Ask Claude to push it to GitHub and ship it wednesday. Thanks!";
            let lines = [
                json!({"type": "user", "message": {"role": "assistant", "content": "x"}}),
                json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": sent}]}}),
            ];
            std::fs::write(log, lines.map(|l| l.to_string() + "\n").concat()).unwrap();
        })
    };
    let notes: Mutex<Vec<String>> = Mutex::default();
    watch_with(&lf, pasted, None, 1.5, &|m| notes.lock().unwrap().push(m), w);
    writer.join().unwrap();
    // Claw -> Claude is already in words.txt; the fake model skips "wednesday" (lowercase).
    assert_eq!(*notes.lock().unwrap(), ["Learned: git hub → GitHub"]);
    assert!(lf.misheard_hints().contains(&("GitHub".to_string(), vec!["git hub".to_string()])));
    assert_eq!(*judged.lock().unwrap(), 1);
}

#[test]
fn watch_stops_at_the_next_dictation() {
    let home = temp("next");
    let lf = localflow(temp("next-config"), &[]);
    let last = home.join("last.json");
    remember_at(&last, "first", 1.0).unwrap();
    remember_at(&last, "second", 2.0).unwrap();
    assert_eq!(last_dictation_at(&last), Some(("second".into(), 2.0)));
    let w = Watch { sent: SentMessages::watching(&home, vec![]), last_file: last, poll: Duration::from_millis(10),
                    limit: Duration::from_secs(10), text_box: || None };
    let start = std::time::Instant::now();
    watch_with(&lf, "first", None, 1.0, &|_| panic!("nothing to learn"), w);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn learns_from_the_text_box_once_you_move_on() {
    // macOS / Windows: the box holds your fixed text, then it's sent (cleared).
    static SHOWN: Mutex<usize> = Mutex::new(0);
    let (url, _) = fake_api();
    let home = temp("box");
    let lf = localflow(temp("box-config"), &[("LOCALFLOW_API_URL", &url), ("LOCALFLOW_MODEL", "m"), ("LOCALFLOW_API_KEY", "k")]);
    let pasted = "Deploy it to vercell today.";
    let last = home.join("last.json");
    remember_at(&last, pasted, 3.0).unwrap();
    let w = Watch {
        sent: SentMessages::watching(&home, vec![]),
        last_file: last,
        poll: Duration::from_millis(10),
        limit: Duration::from_secs(10),
        text_box: || {
            let mut n = SHOWN.lock().unwrap();
            *n += 1;
            (*n <= 3).then(|| (7, "Deploy it to Vercel today.".to_string()))
        },
    };
    let notes: Mutex<Vec<String>> = Mutex::default();
    watch_with(&lf, pasted, Some(7), 3.0, &|m| notes.lock().unwrap().push(m), w);
    assert_eq!(*notes.lock().unwrap(), ["Learned: vercell → Vercel"]);
}

#[test]
fn sent_messages_from_cursor_chat() {
    let dir = temp("cursor");
    let db_path = dir.join("state.vscdb");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute("CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)", []).unwrap();
    let put = |k: &str, v: Value| db.execute("INSERT INTO cursorDiskKV VALUES (?1, ?2)", (k, v.to_string())).unwrap();
    put("bubbleId:a:1", json!({"type": 1, "text": "old"}));
    let mut sent = SentMessages::watching(&dir, vec![db_path, dir.join("no-such.vscdb")]);
    assert!(sent.take_new().is_empty());
    put("bubbleId:a:2", json!({"type": 1, "text": "hi"}));
    put("bubbleId:a:3", json!({"type": 2, "text": "ai"}));
    put("composerData:x", json!({"type": 1, "text": "not a bubble"}));
    assert_eq!(sent.take_new(), ["hi"]);
    assert!(sent.take_new().is_empty());
}

#[test]
fn codex_transcripts_and_partial_lines() {
    let home = temp("codex");
    let mut sent = SentMessages::watching(&home, vec![]);
    let log = home.join(".codex/sessions/2026/10/20/rollout.jsonl");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let line = json!({"payload": {"role": "user", "content": [{"type": "input_text", "text": "fix it"}]}}).to_string();
    std::fs::write(&log, format!("{line}\n{{\"payload\": ")).unwrap(); // the second line is still being written
    assert_eq!(sent.take_new(), ["fix it"]);
    let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
    writeln!(f, "{{\"role\": \"user\", \"content\": \"and this\"}}}}").unwrap();
    assert_eq!(sent.take_new(), ["and this"]);
}

#[test]
fn learn_key() {
    let config = temp("key");
    let lf = localflow(config.clone(), &[]);
    let last = "Push it to git hub and ask Claw.";
    assert_eq!(learn_from(&lf, last, vec!["".into(), "Something else entirely here".into()]), "Select the text you fixed");
    assert_eq!(learn_from(&lf, last, vec![last.into()]), "No changed words found");
    assert_eq!(learn_from(&lf, last, vec!["  ".into(), "Push it to GitHub and ask Claw.".into()]), "Learned: git hub → GitHub");
    assert_eq!(learn_from(&lf, last, vec!["Push it to GitHub and ask Claw.".into()]), "Already learned");
    assert!(std::fs::read_to_string(config.join("words.txt")).unwrap().contains("GitHub: often heard as \"git hub\""));
}

#[test]
fn messages() {
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(learned_message(&[pair("Claw", "Claude")]), "Learned: Claw → Claude");
    assert_eq!(learned_message(&[pair("Claw", "Claude"), pair("x", "y")]), "Learned 2 words");
    assert_eq!(learned_message(&[pair("a very long misheard phrase", "Something")]), "Learned 1 word");
}
