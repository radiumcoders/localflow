//! Diagrams drawn by Claude Haiku 5.5 with your Anthropic API key (beta). The
//! answer is held to a JSON schema (the API's structured outputs): the
//! diagram's parts, not Mermaid. localflow writes the Mermaid itself, so the
//! labels are always safe to draw. Only what you described for the diagram is
//! sent, and only when Haiku is chosen in the settings.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::{Value, json};

use crate::diagram::{Diagram, HAIKU, Kind, Request, Shape, flowchart};
use crate::{Error, Localflow, Result, Secret, err};

const URL: &str = "https://api.anthropic.com/v1/messages";

const SYSTEM: &str = "You turn a dictated description into a diagram's parts.
The dictation is speech: skip filler words, and where the speaker corrects themselves (\"no wait\", \"actually\"), keep only the correction.
Draw only the steps, ideas, people and systems it gives, in its order, each once. Add nothing.
Kinds: flowchart for steps, flows, processes and decisions; mindmap for a topic and its subtopics with no order; sequence for messages that people or systems send each other, in order.
Labels are short (2-6 words), in the speaker's words, without brackets or quotes.
flowchart: nodes with ids, each a step, a decision (a short question) or start_end (where it starts or finishes); edges from id to id, a decision's edges labeled with the answers (Yes, No, ...). direction is LR only when the speaker says left to right, else TD.
mindmap: title is the central topic; nodes are its subtopics, each with the id of its parent node, or an empty parent when it's directly under the title. At most three levels.
sequence: messages in order, from and to are the people or systems by name; reply is true for an answer going back.
Leave the fields another kind uses empty.";

/// The answer's schema: the parts of any of the three kinds (no recursion,
/// which structured outputs don't take: a mind map's nodes name their parent).
fn schema(kind: Option<Kind>) -> Value {
    let kinds: Vec<&str> = match kind {
        Some(Kind::Flowchart) => vec!["flowchart"],
        Some(Kind::Mindmap) => vec!["mindmap"],
        Some(Kind::Sequence) => vec!["sequence"],
        // Haiku tells a sequence from a process (the small local models don't).
        None => vec!["flowchart", "mindmap", "sequence"],
    };
    let object = |properties: Value| {
        let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
        json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
    };
    let string = json!({"type": "string"});
    object(json!({
        "kind": {"type": "string", "enum": kinds},
        "direction": {"type": "string", "enum": ["TD", "LR"]},
        "title": string,
        "nodes": {"type": "array", "items": object(json!({
            "id": string,
            "label": string,
            "shape": {"type": "string", "enum": ["step", "decision", "start_end"]},
            "parent": string,
        }))},
        "edges": {"type": "array", "items": object(json!({"from": string, "to": string, "label": string}))},
        "messages": {"type": "array", "items": object(json!({
            "from": string, "to": string, "text": string, "reply": {"type": "boolean"},
        }))},
    }))
}

/// The Messages API request: Haiku 5.5 only, thinking off at low effort
/// (a diagram is a quick job), the answer held to the schema.
pub(crate) fn body(request: &Request) -> Value {
    let ask = match request.kind {
        Some(kind) => format!("Draw a {}.\n<dictation>{}</dictation>", kind.name(), request.about),
        None => format!("<dictation>{}</dictation>", request.about),
    };
    json!({
        "model": HAIKU,
        "max_tokens": 4096,
        "thinking": {"type": "disabled"},
        "output_config": {"effort": "low", "format": {"type": "json_schema", "schema": schema(request.kind)}},
        "system": SYSTEM,
        "messages": [{"role": "user", "content": ask}],
    })
}

/// A label Mermaid draws as text: no brackets, quotes, pipes or other
/// characters it reads as syntax, one line, not too long.
fn label(text: &str) -> String {
    let text: String = text
        .chars()
        .map(|c| if "[]{}()<>|\"`;\\".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let text = text.replace("%%", "%").split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text: String = text.trim_start_matches(['-', '*', '#', ':', '/', '.', ' ']).trim().to_string();
    if text.chars().count() > 60 {
        text = text.chars().take(59).collect::<String>().trim_end().to_string() + "…";
    }
    text
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The diagram in Haiku's answer (the schema's JSON), or None when there's
/// too little of one.
pub(crate) fn diagram(answer: &Value) -> Option<Diagram> {
    match text(answer, "kind") {
        "flowchart" => flowchart_from(answer).map(|mermaid| Diagram::new(Kind::Flowchart, mermaid)),
        "mindmap" => mindmap_from(answer).map(|mermaid| Diagram::new(Kind::Mindmap, mermaid)),
        "sequence" => sequence_from(answer).map(|mermaid| Diagram::new(Kind::Sequence, mermaid)),
        _ => None,
    }
}

fn flowchart_from(answer: &Value) -> Option<String> {
    let (mut labels, mut index) = (vec![], HashMap::new());
    for node in items(answer, "nodes") {
        let name = label(text(node, "label"));
        if name.is_empty() || index.contains_key(text(node, "id")) {
            continue;
        }
        let shape = match text(node, "shape") {
            "decision" => Shape::Decision,
            "start_end" => Shape::Terminal,
            _ => Shape::Step,
        };
        index.insert(text(node, "id").to_string(), labels.len());
        labels.push((name, shape));
    }
    let mut seen = HashSet::new();
    let edges: Vec<(usize, Option<String>, usize)> = items(answer, "edges")
        .iter()
        .filter_map(|e| Some((*index.get(text(e, "from"))?, Some(label(text(e, "label"))), *index.get(text(e, "to"))?)))
        .filter(|(a, _, b)| seen.insert((*a, *b)))
        .collect();
    let direction = if text(answer, "direction") == "LR" { "LR" } else { "TD" };
    flowchart(direction, &labels, &edges)
}

fn mindmap_from(answer: &Value) -> Option<String> {
    let nodes = items(answer, "nodes");
    let index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (text(n, "id"), i)).collect();
    // Each node's parent node; none (the title) when it names no other node.
    let parents: Vec<Option<usize>> =
        nodes.iter().enumerate().map(|(i, n)| index.get(text(n, "parent")).copied().filter(|&p| p != i)).collect();
    let mut title = label(text(answer, "title"));
    if title.is_empty() {
        title = "Ideas".into();
    }
    let mut out = format!("mindmap\n  root(({title}))\n");
    let mut drawn = vec![false; nodes.len()];
    // Depth first from the title, three levels at most; each node once.
    let mut stack: Vec<(Option<usize>, usize)> = vec![(None, 0)];
    let mut order = vec![];
    while let Some((parent, depth)) = stack.pop() {
        if let Some(p) = parent {
            order.push((p, depth));
        }
        if depth == 3 {
            continue;
        }
        let children: Vec<usize> =
            (0..nodes.len()).filter(|&i| parents[i] == parent && !drawn[i] && !label(text(&nodes[i], "label")).is_empty()).collect();
        for &c in &children {
            drawn[c] = true;
        }
        stack.extend(children.into_iter().rev().map(|c| (Some(c), depth + 1)));
    }
    for (i, depth) in &order {
        out += &format!("{}{}\n", "  ".repeat(depth + 1), label(text(&nodes[*i], "label")));
    }
    (!order.is_empty()).then_some(out)
}

fn sequence_from(answer: &Value) -> Option<String> {
    let mut names: Vec<String> = vec![];
    let mut lines = vec![];
    let alias = |name: &str, names: &mut Vec<String>| -> Option<String> {
        let name = label(name);
        if name.is_empty() {
            return None;
        }
        let i = names.iter().position(|n| n.eq_ignore_ascii_case(&name)).unwrap_or_else(|| {
            names.push(name);
            names.len() - 1
        });
        Some(format!("P{}", i + 1))
    };
    for m in items(answer, "messages") {
        let (Some(from), Some(to)) = (alias(text(m, "from"), &mut names), alias(text(m, "to"), &mut names)) else { continue };
        let arrow = if m["reply"].as_bool() == Some(true) { "-->>" } else { "->>" };
        let what = label(text(m, "text"));
        lines.push(format!("    {from}{arrow}{to}: {}\n", if what.is_empty() { "…".into() } else { what }));
    }
    if lines.is_empty() {
        return None;
    }
    let mut out = String::from("sequenceDiagram\n");
    for (i, name) in names.iter().enumerate() {
        out += &format!("    participant P{} as {name}\n", i + 1);
    }
    Some(out + &lines.concat())
}

/// One Messages API request with your key: Claude's answer, or why there's
/// none.
pub(crate) fn post(key: &Secret, body: &Value, timeout: Duration) -> Result<Value> {
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let response = agent.post(URL).set("x-api-key", &key.0).set("anthropic-version", "2026-10-21").send_json(body);
    match response {
        Ok(r) => r.into_json().map_err(|e| Error(format!("Claude's answer didn't read: {e}"))),
        Err(ureq::Error::Status(code, r)) => {
            let detail: Value = r.into_json().unwrap_or_default();
            let why = detail["error"]["message"].as_str().unwrap_or("no detail");
            err(match code {
                401 => "the Anthropic API key was refused".to_string(),
                _ => format!("Claude answered HTTP {code}: {}", why.chars().take(200).collect::<String>()),
            })
        }
        Err(e) => err(format!("couldn't reach Claude: {e}")),
    }
}

/// The text of Claude's answer.
pub(crate) fn answer_text(answer: &Value) -> String {
    items(answer, "content").iter().filter(|b| b["type"] == "text").map(|b| text(b, "text")).collect()
}

impl Localflow {
    /// Draw with Claude Haiku 5.5: an error when there's no key, the API
    /// can't be reached, or it declines.
    pub(crate) fn haiku_diagram(&self, request: &Request) -> Result<Diagram> {
        let Some(key) = &self.cfg.anthropic_key else {
            return err("no Anthropic API key (Settings > Visualize)");
        };
        let answer = post(key, &body(request), Duration::from_secs(30))?;
        match answer["stop_reason"].as_str() {
            Some("refusal") => return err("Claude declined to draw that"),
            Some("max_tokens") => return err("Claude's diagram was cut off"),
            _ => {}
        }
        let parts: Value = serde_json::from_str(&answer_text(&answer)).map_err(|e| Error(format!("Claude's diagram didn't read: {e}")))?;
        diagram(&parts).map(|d| d.laid_out_for(request)).ok_or_else(|| Error("couldn't draw that".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_haiku_with_a_schema() {
        let b = body(&Request { kind: Some(Kind::Mindmap), visualize: true, about: "my trip".into() });
        assert_eq!(b["model"], "claude-haiku-5-5");
        assert_eq!(b["thinking"]["type"], "disabled");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        let schema = &b["output_config"]["format"]["schema"];
        assert_eq!(schema["properties"]["kind"]["enum"], json!(["mindmap"]));
        // Structured outputs: every object closed, every field required.
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["nodes"]["items"]["additionalProperties"], false);
        assert_eq!(schema["required"].as_array().unwrap().len(), 6);
        assert!(b.get("temperature").is_none());
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("<dictation>my trip</dictation>"));
        let auto = body(&Request { kind: None, visualize: true, about: "x".into() });
        assert_eq!(auto["output_config"]["format"]["schema"]["properties"]["kind"]["enum"], json!(["flowchart", "mindmap", "sequence"]));
    }

    #[test]
    fn flowchart_from_parts() {
        let answer = json!({"kind": "flowchart", "direction": "TD", "title": "", "messages": [],
            "nodes": [
                {"id": "a", "label": "Sign in", "shape": "start_end", "parent": ""},
                {"id": "b", "label": "Password [right]?", "shape": "decision", "parent": ""},
                {"id": "c", "label": "Show \"dashboard\"", "shape": "step", "parent": ""},
                {"id": "d", "label": "Unused", "shape": "step", "parent": ""},
                {"id": "a", "label": "Duplicate id", "shape": "step", "parent": ""}],
            "edges": [{"from": "a", "to": "b", "label": ""}, {"from": "b", "to": "c", "label": "Yes | sure"},
                      {"from": "b", "to": "c", "label": "again"}, {"from": "b", "to": "zz", "label": ""}]});
        let d = diagram(&answer).unwrap();
        assert_eq!(d.kind, Kind::Flowchart);
        assert_eq!(d.mermaid, "flowchart TD\n    N1([\"Sign in\"])\n    N2{\"Password right ?\"}\n    N3[\"Show dashboard\"]\n    \
                               N1 --> N2\n    N2 -->|Yes sure| N3\n");
    }

    #[test]
    fn mindmap_from_parents() {
        let answer = json!({"kind": "mindmap", "direction": "TD", "title": "Japan (trip)", "edges": [], "messages": [],
            "nodes": [
                {"id": "t", "label": "Tokyo", "shape": "step", "parent": ""},
                {"id": "s", "label": "Shibuya", "shape": "step", "parent": "t"},
                {"id": "k", "label": "Kyoto", "shape": "step", "parent": "Japan"},
                {"id": "x", "label": "Loop", "shape": "step", "parent": "x"},
                {"id": "y", "label": "Orphan's child", "shape": "step", "parent": "nowhere"}]});
        assert_eq!(diagram(&answer).unwrap().mermaid,
                   "mindmap\n  root((Japan trip))\n    Tokyo\n      Shibuya\n    Kyoto\n    Loop\n    Orphan's child\n");
    }

    #[test]
    fn sequence_with_names() {
        let answer = json!({"kind": "sequence", "direction": "TD", "title": "", "nodes": [], "edges": [],
            "messages": [{"from": "Browser", "to": "API server", "text": "GET /user", "reply": false},
                         {"from": "api server", "to": "Browser", "text": "the JSON", "reply": true}]});
        assert_eq!(diagram(&answer).unwrap().mermaid,
                   "sequenceDiagram\n    participant P1 as Browser\n    participant P2 as API server\n    \
                    P1->>P2: GET /user\n    P2-->>P1: the JSON\n");
        assert_eq!(diagram(&json!({"kind": "sequence", "messages": []})), None);
        assert_eq!(diagram(&json!({"kind": "pie"})), None);
    }

    #[test]
    fn labels_are_only_text() {
        assert_eq!(label("  A (big) [step] {x} | \"y\" `z`; 100%% done "), "A big step x y z 100% done");
        assert_eq!(label("- * item"), "item");
        assert_eq!(label(&"word ".repeat(30)).chars().count(), 60);
    }
}
