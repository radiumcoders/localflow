//! Visualize: a dictation that asks for a diagram ("draw a flowchart of how
//! we deploy: ...") becomes one. The model writes Mermaid, held by a grammar
//! to the few forms localflow draws (a flowchart, a mind map, a sequence
//! diagram), and `tidy` keeps what it meant: a small model that doesn't know
//! when to stop goes round in circles, and the circles are cut off.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::local::{ModelFile, load_model, models_dir};
use crate::{Config, Error, Localflow, Result, err};

/// The models that draw: Qwen3.5 (Apache-2.0), as Unsloth quantized it,
/// pinned to a revision. The first is the default.
pub const MODELS: [ModelFile; 2] = [
    ModelFile {
        id: "qwen3.5-2b",
        name: "Qwen3.5 2B, for diagrams",
        file: "Qwen3.5-2B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf",
        size: 1_280_835_840,
        sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
    },
    ModelFile {
        id: "qwen3.5-4b",
        name: "Qwen3.5 4B, for diagrams",
        file: "Qwen3.5-4B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/e87f176479d0855a907a41277aca2f8ee7a09523/Qwen3.5-4B-Q4_K_M.gguf",
        size: 2_740_937_888,
        sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
    },
];

impl Config {
    /// Diagrams are drawn (the setting isn't off).
    pub fn diagrams(&self) -> bool {
        self.diagram_model != "off"
    }

    /// The diagram model's download, when it's one of ours.
    pub fn diagram_download(&self) -> Option<&'static ModelFile> {
        MODELS.iter().find(|m| m.id == self.diagram_model)
    }

    /// The diagram model's file: ours in the models folder, or the .gguf named.
    pub fn diagram_model_file(&self) -> std::path::PathBuf {
        match self.diagram_download() {
            Some(m) => models_dir().join(m.file),
            None => std::path::PathBuf::from(&self.diagram_model),
        }
    }

    /// Diagrams are on and their model is here (downloaded and complete).
    pub fn diagram_model_installed(&self) -> bool {
        let path = self.diagram_model_file();
        self.diagrams()
            && match self.diagram_download() {
                Some(m) => std::fs::metadata(&path).is_ok_and(|f| f.len() == m.size),
                None => path.exists(),
            }
    }
}

/// The kind of diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Steps and decisions.
    Flowchart,
    /// A topic, its subtopics and theirs.
    Mindmap,
    /// Messages between people or systems, in order.
    Sequence,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Flowchart => "flowchart",
            Kind::Mindmap => "mind map",
            Kind::Sequence => "sequence diagram",
        }
    }
}

/// A dictation that asks for a diagram: what to draw, and the kind when it
/// was named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: Option<Kind>,
    /// It began with "visualize" (not "draw a flowchart of", which a coding
    /// agent may be asked too).
    pub visualize: bool,
    /// The description, without the words that asked for a diagram.
    pub about: String,
}

/// "Visualize …", "draw a flowchart of …", "mind map: …" at the start.
static ASK: LazyLock<Regex> = LazyLock::new(|| {
    let filler = r"(?:(?:okay|ok|so|um|uh|hey|please|can you|could you|now)[\s,]+)*";
    let kind = |n: u8| {
        format!(r"(?P<kind{n}>flow\s?charts?|flow\s?diagrams?|process\s?diagrams?|mind\s?maps?|sequence\s?diagrams?|diagrams?)")
    };
    let verb = r"(?:draw|make|create|generate|build|sketch|do|give|show|turn (?:this|that) into)";
    let then = r"(?:\s*[:,.\-]|\s+(?:of|for|about|showing|that\s+shows|where|on|with|explaining|to\s+show)\b)";
    Regex::new(&format!(
        r"(?ix)^\W*{filler}
        (?:
            (?P<visualize>visuali[sz]e)(?:\s+(?:this|that|it))?(?:\s+as\s+an?\s+{k1})?(?:\s*[:,.\-]|\s+)
          | {verb}\s+(?:me\s+|us\s+)?(?:an?\s+|the\s+)?(?:quick\s+|simple\s+|little\s+)?{k2}(?:{then}|\s+)
          | {k3}(?:\s*[:,\-]|\s+of\b)
        )",
        k1 = kind(1),
        k2 = kind(2),
        k3 = kind(3),
    ))
    .unwrap()
});

/// The diagram a dictation asks for, or None when it's a dictation. Only the
/// start counts, and something to draw must follow: "I drew a flowchart" and
/// "visualize" alone are dictations; a kind said first needs "of" or a pause
/// after it ("Diagrams are fun" is prose, "Flowchart: …" a diagram).
pub fn request(said: &str) -> Option<Request> {
    let said = said.replace("<quiet>", "").replace("</quiet>", "");
    let m = ASK.captures(&said)?;
    let kind = ["kind1", "kind2", "kind3"].iter().find_map(|k| m.name(k));
    let kind = match kind.map(|k| k.as_str().to_lowercase().replace(char::is_whitespace, "")).as_deref() {
        Some(k) if k.starts_with("mindmap") => Some(Kind::Mindmap),
        Some(k) if k.starts_with("sequence") => Some(Kind::Sequence),
        Some(k) if k.starts_with("flow") || k.starts_with("process") => Some(Kind::Flowchart),
        _ => None,
    };
    let about = said[m.get(0).unwrap().end()..].trim_matches(|c: char| c.is_whitespace() || ",.:;-".contains(c));
    if about.split_whitespace().count() < 3 {
        return None;
    }
    Some(Request { kind, visualize: m.name("visualize").is_some(), about: about.to_string() })
}

/// The instruction and examples the model draws from.
const SYSTEM: &str = "Draw the dictation as a Mermaid diagram: only the steps, ideas and names it gives, in its order, each once.
Steps, flows and decisions: a flowchart. A mindmap only for topics and their subtopics, with no order or steps. Messages between people or systems: a sequenceDiagram.
Labels are short (2-5 words). A decision is a {question} with labeled edges.

Example: \"people sign in and if the password is right they see the dashboard if not we show an error\"
flowchart TD
    A([Sign in]) --> B{Password right?}
    B -->|Yes| C[Show dashboard]
    B -->|No| D[Show error]

Example: \"a mind map of the launch with marketing which has the blog post and the video and engineering which has the release\"
mindmap
  root((Launch))
    Marketing
      Blog post
      Video
    Engineering
      Release

Example: \"the app asks the server for the profile and the server sends it back\"
sequenceDiagram
    App->>Server: Get profile
    Server-->>App: Profile";

/// What the model may write (GBNF), per kind; `root` picks.
const RULES: &str = r#"
flowchart ::= "flowchart " ("TD" | "LR") ("\n    " fline){1,30} "\n"?
fline ::= node (" -->" ("|" text "|")? " " node)?
node ::= id ("[" text "]" | "{" text "}" | "([" text "])")?
id ::= [A-Z] [A-Z0-9]?
mindmap ::= "mindmap\n  root((" text "))" ("\n" ("    " | "      " | "        ") text){1,40} "\n"?
sequence ::= "sequenceDiagram" ("\n    " name ("->>" | "-->>") name ": " text){1,30} "\n"?
name ::= [A-Za-z] [A-Za-z0-9_]{0,15}
text ::= [A-Za-z0-9] [A-Za-z0-9 ,.'?!&/:+#%=-]{0,39}
"#;

pub fn grammar(kind: Option<Kind>) -> String {
    let root = match kind {
        // A sequence diagram only when asked for: small models reach for it
        // for any process with people in it, and draw it badly.
        None => "flowchart | mindmap",
        Some(Kind::Flowchart) => "flowchart",
        Some(Kind::Mindmap) => "mindmap",
        Some(Kind::Sequence) => "sequence",
    };
    format!("root ::= {root}{RULES}")
}

/// The prompt, in Qwen3.5's chat format with thinking off.
pub fn prompt(request: &Request) -> String {
    let ask = match request.kind {
        Some(kind) => format!("Draw a {}: {}", kind.name(), request.about),
        None => request.about.clone(),
    };
    format!(
        "<|im_start|>system\n{SYSTEM}<|im_end|>\n<|im_start|>user\n<dictation>{ask}</dictation><|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
}

/// A drawn diagram: Mermaid that renders, and its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagram {
    pub kind: Kind,
    pub mermaid: String,
}

/// A label for Mermaid: in quotes, so its punctuation is only text.
fn quoted(label: &str) -> String {
    format!("\"{}\"", label.trim().replace('"', "'"))
}

fn key(label: &str) -> String {
    label.trim().trim_end_matches(['.', '?', '!']).to_lowercase()
}

static NODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^([A-Z][A-Z0-9]?)(?:\(\[(.+?)\]\)|\[(.+?)\]|\{(.+?)\})?$"#).unwrap());
static EDGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?) -->(?:\|(.+?)\|)? (.+)$").unwrap());
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:\d+[.)]|[-*])\s+").unwrap());
static MESSAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9_]*)(->>|-->>)([A-Za-z][A-Za-z0-9_]*): (.+)$").unwrap());

/// The flowchart the model meant: each step once (a label written again is
/// the same step), an id given a second label keeps its first, and the
/// drawing stops where it starts going round (an edge drawn again, a step
/// pointing at itself) or adds steps that are only loose ends.
fn tidy_flowchart(lines: &[&str], direction: &str) -> Option<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Shape {
        Step,
        Decision,
        Terminal,
    }
    let mut labels: Vec<(String, Shape)> = vec![]; // by node number
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_label: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<(usize, Option<String>, usize)> = vec![];
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut node = |text: &str, labels: &mut Vec<(String, Shape)>| -> Option<usize> {
        let c = NODE.captures(text.trim())?;
        let id = c[1].to_string();
        let given = c.get(2).map(|m| (m.as_str(), Shape::Terminal))
            .or(c.get(3).map(|m| (m.as_str(), Shape::Step)))
            .or(c.get(4).map(|m| (m.as_str(), Shape::Decision)));
        if let Some(&n) = by_id.get(&id) {
            return Some(n); // a new label on a known id is the model losing track: keep the first
        }
        let (label, shape) = given.map_or((id.clone(), Shape::Step), |(l, s)| (l.trim().to_string(), s));
        if let Some(&n) = by_label.get(&key(&label)) {
            by_id.insert(id, n);
            return Some(n);
        }
        by_label.insert(key(&label), labels.len());
        by_id.insert(id, labels.len());
        labels.push((label, shape));
        let n = labels.len() - 1;
        Some(n)
    };
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let before = labels.len();
        if let Some(c) = EDGE.captures(line) {
            let (Some(from), Some(to)) = (node(&c[1], &mut labels), node(&c[3], &mut labels)) else { break };
            if from == to || !seen.insert((from, to)) {
                labels.truncate(before);
                break; // going round
            }
            edges.push((from, c.get(2).map(|m| m.as_str().trim().to_string()), to));
        } else if node(line, &mut labels).is_none() {
            break;
        }
        if labels.len() > 30 {
            break;
        }
    }
    // Only the nodes still in use, in their order.
    let used: HashSet<usize> = edges.iter().flat_map(|&(a, _, b)| [a, b]).collect();
    let keep: Vec<usize> = (0..labels.len()).filter(|n| used.contains(n) || edges.is_empty()).collect();
    if keep.len() < 2 {
        return None;
    }
    let id = |n: usize| format!("N{}", keep.iter().position(|&k| k == n).unwrap() + 1);
    let mut out = format!("flowchart {direction}\n");
    for &n in &keep {
        let (label, shape) = &labels[n];
        let label = quoted(label);
        out += &match shape {
            Shape::Step => format!("    {}[{label}]\n", id(n)),
            Shape::Decision => format!("    {}{{{label}}}\n", id(n)),
            Shape::Terminal => format!("    {}([{label}])\n", id(n)),
        };
    }
    for (a, label, b) in &edges {
        match label {
            // (The grammar keeps | out of labels; quotes here would be drawn.)
            Some(l) => out += &format!("    {} -->|{}| {}\n", id(*a), l.trim(), id(*b)),
            None => out += &format!("    {} --> {}\n", id(*a), id(*b)),
        }
    }
    Some(out)
}

/// The mind map, up to the first topic said again (where it goes round).
fn tidy_mindmap(root: &str, lines: &[&str]) -> Option<String> {
    let mut seen: HashSet<String> = HashSet::from([key(root)]);
    let mut out = format!("mindmap\n  root(({}))\n", root.trim());
    let mut count = 0;
    let mut depth_above = 0; // a branch can only go one level deeper than the line before
    for line in lines {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if !seen.insert(key(text)) {
            break;
        }
        let indent = line.len() - line.trim_start().len();
        let depth = ((indent.max(4) - 4) / 2 + 1).min(depth_above + 1);
        depth_above = depth;
        let text = NUMBERED.replace(text, "");
        out += &format!("{}{}\n", "  ".repeat(depth + 1), text.trim_end_matches(':'));
        count += 1;
    }
    (count >= 1).then_some(out)
}

/// The sequence, up to the first message sent again.
fn tidy_sequence(lines: &[&str]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut out = String::from("sequenceDiagram\n");
    let mut count = 0;
    for line in lines {
        let Some(c) = MESSAGE.captures(line.trim()) else { break };
        if !seen.insert((c[1].to_string(), c[3].to_string(), key(&c[4]))) {
            break;
        }
        out += &format!("    {}{}{}: {}\n", &c[1], &c[2], &c[3], c[4].trim());
        count += 1;
    }
    (count >= 1).then_some(out)
}

/// The diagram in the model's answer, tidied (see the functions above), or
/// None when there's too little of one.
pub fn tidy(answer: &str) -> Option<Diagram> {
    let answer = answer.trim().trim_start_matches("```mermaid").trim_matches('`').trim();
    let mut lines = answer.lines();
    let head = lines.next()?.trim();
    let rest: Vec<&str> = lines.collect();
    if let Some(direction) = head.strip_prefix("flowchart ") {
        let direction = if direction.trim() == "LR" { "LR" } else { "TD" };
        return tidy_flowchart(&rest, direction).map(|mermaid| Diagram { kind: Kind::Flowchart, mermaid });
    }
    if head == "mindmap" {
        let (root, branches) = rest.split_first()?;
        let root = root.trim().strip_prefix("root((")?.strip_suffix("))")?;
        return tidy_mindmap(root, branches).map(|mermaid| Diagram { kind: Kind::Mindmap, mermaid });
    }
    if head == "sequenceDiagram" {
        return tidy_sequence(&rest).map(|mermaid| Diagram { kind: Kind::Sequence, mermaid });
    }
    None
}

impl Localflow {
    /// Draw what a dictation asked for, with the diagram model from the
    /// settings. The cleanup model, loaded while you spoke, makes room for it.
    pub fn diagram(&self, request: &Request) -> Result<Diagram> {
        if !self.cfg.diagram_model_installed() {
            return err("the diagram model isn't downloaded yet");
        }
        *self.engine.lock().unwrap() = None;
        let mut engine = self.diagram_engine.lock().unwrap();
        if engine.is_none() {
            *engine = Some(load_model(&self.cfg.diagram_model_file(), self.cfg.cleanup_gpu)?);
        }
        // Cut off at the limit is fine: tidy keeps what came before the circles.
        let (answer, _) =
            engine.as_mut().unwrap().complete_with(&prompt(request), Some(&grammar(request.kind)), 900).map_err(Error)?;
        tidy(&answer).ok_or_else(|| Error("couldn't draw that".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_a_diagram() {
        let req = |s: &str| request(s).map(|r| (r.kind, r.about));
        assert!(request("Visualize the deploy: test, merge, ship").unwrap().visualize);
        assert!(!request("Draw a flowchart of the deploy: test, merge, ship").unwrap().visualize);
        assert_eq!(req("Create a sequence diagram the browser calls the API"),
                   Some((Some(Kind::Sequence), "the browser calls the API".into())));
        assert_eq!(req("Visualize the signup flow: user signs up, we email them, they log in."),
                   Some((None, "the signup flow: user signs up, we email them, they log in".into())));
        assert_eq!(req("Draw a flowchart of how we deploy: tests, review, merge."),
                   Some((Some(Kind::Flowchart), "how we deploy: tests, review, merge".into())));
        assert_eq!(req("Okay, make me a mind map of my trip with Tokyo and Kyoto"),
                   Some((Some(Kind::Mindmap), "my trip with Tokyo and Kyoto".into())));
        assert_eq!(req("Create a sequence diagram, the browser calls the API and the API calls the database."),
                   Some((Some(Kind::Sequence), "the browser calls the API and the API calls the database".into())));
        assert_eq!(req("Flow chart: wake up, shower, take the train."),
                   Some((Some(Kind::Flowchart), "wake up, shower, take the train".into())));
        assert_eq!(req("Diagram of the login flow with the password check"),
                   Some((None, "the login flow with the password check".into())));
        assert_eq!(req("visualise this as a flowchart we ship on monday and test on tuesday"),
                   Some((Some(Kind::Flowchart), "we ship on monday and test on tuesday".into())));
        for dictation in ["I drew a flowchart of the deploy yesterday.", "Visualize.", "Draw a flowchart.",
                          "Diagrams are a great way to explain things to people.", "Make the diagram bigger",
                          "The flowchart is wrong, fix the second step please.", "Let's visualize success"] {
            assert_eq!(req(dictation), None, "{dictation}");
        }
    }

    #[test]
    fn grammar_per_kind() {
        assert!(grammar(None).starts_with("root ::= flowchart | mindmap\n"));
        assert!(grammar(Some(Kind::Mindmap)).starts_with("root ::= mindmap\n"));
    }

    #[test]
    fn flowchart_stops_going_round() {
        let answer = "flowchart TD\n    A([Sign in]) --> B{Password right?}\n    B -->|Yes| C[Show dashboard]\n    \
                      B -->|No| D[Show error]\n    D --> A\n    C --> E[Show dashboard]\n    E --> F[Done]\n    \
                      A --> B\n    B --> C\n";
        let d = tidy(answer).unwrap();
        assert_eq!(d.kind, Kind::Flowchart);
        assert_eq!(d.mermaid, "flowchart TD\n    N1([\"Sign in\"])\n    N2{\"Password right?\"}\n    \
                               N3[\"Show dashboard\"]\n    N4[\"Show error\"]\n    N1 --> N2\n    \
                               N2 -->|Yes| N3\n    N2 -->|No| N4\n    N4 --> N1\n");
        // An id given a new label keeps its first; the edge back to it is the loop.
        let answer = "flowchart LR\n    A[Open PR] --> B[Run tests]\n    B --> C[Merge]\n    C --> A[Something else]\n    \
                      A --> B\n";
        assert_eq!(tidy(answer).unwrap().mermaid, "flowchart LR\n    N1[\"Open PR\"]\n    N2[\"Run tests\"]\n    \
                                                    N3[\"Merge\"]\n    N1 --> N2\n    N2 --> N3\n    N3 --> N1\n");
        assert_eq!(tidy("flowchart TD\n    A[Only one]\n"), None);
    }

    #[test]
    fn mindmap_and_sequence() {
        let d = tidy("mindmap\n  root((Japan))\n    1. Tokyo\n      Shibuya\n    Kyoto\n        Temples\n    Osaka\n    \
                      Kyoto\n    Osaka\n").unwrap();
        assert_eq!(d.mermaid, "mindmap\n  root((Japan))\n    Tokyo\n      Shibuya\n    Kyoto\n      Temples\n    Osaka\n");
        let d = tidy("sequenceDiagram\n    App->>API: Get user\n    API-->>App: User\n    App->>API: Get user\n").unwrap();
        assert_eq!(d.mermaid, "sequenceDiagram\n    App->>API: Get user\n    API-->>App: User\n");
        assert_eq!(tidy("Here is your diagram"), None);
    }
}
