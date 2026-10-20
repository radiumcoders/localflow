//! A fake cleanup API for tests: Anthropic Messages and OpenAI-compatible
//! chat. It "cleans up" by capitalizing the dictation and adding a period.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use regex::Regex;
use serde_json::{Value, json};

/// Serve on a free port, in the background. Returns the base URL (…/v1).
pub fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("no port for the fake API");
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = serve(stream);
            });
        }
    });
    url
}

fn serve(mut stream: std::net::TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
    let mut headers = vec![];
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        match line.trim_end().split_once(':') {
            Some((k, v)) => headers.push((k.to_lowercase(), v.trim().to_string())),
            None => break,
        }
    }
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    let (code, answer) = reply(&path, &headers, &serde_json::from_slice(&body).unwrap_or(Value::Null));
    let data = answer.to_string();
    write!(stream, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}",
           data.len())
}

fn reply(path: &str, headers: &[(String, String)], body: &Value) -> (u16, Value) {
    let last = body["messages"].as_array().and_then(|m| m.last()).map(|m| &m["content"]).cloned().unwrap_or_default();
    let last: String = match last {
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect(),
        other => other.as_str().unwrap_or_default().to_string(),
    };
    let text = Regex::new(r"(?s)<dictation>(.*)</dictation>").unwrap().captures(&last).map_or(last.clone(), |c| c[1].to_string());
    let text = Regex::new(r"(?s)<quiet>.*?</quiet>").unwrap().replace_all(&text, " ").split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = text.chars();
    let mut answer = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    answer += chars.as_str().trim_end_matches('.');
    answer += ".";
    if last.contains("CHANGES:") {
        // Judging edits: learn the ones with a capital letter (names).
        let learn: Vec<u32> = Regex::new(r#"(?m)^(\d+)\. ".*" -> "(.*)"$"#)
            .unwrap()
            .captures_iter(&last)
            .filter(|c| c[2].starts_with(char::is_uppercase))
            .filter_map(|c| c[1].parse().ok())
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
