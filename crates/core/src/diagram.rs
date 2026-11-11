//! Visualize: a dictation that asks for a diagram ("draw a flowchart of how
//! we deploy: ...") becomes one. The model writes Mermaid, held by a grammar
//! to the few forms localflow draws (a flowchart, a mind map, a sequence
//! diagram), and `tidy` keeps what it meant: a small model that doesn't know
//! when to stop goes round in circles, and what it repeats is drawn once.

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

/// Claude Haiku 5.5, through the Anthropic API with your key: better
/// diagrams, but what you describe leaves this computer.
pub const HAIKU: &str = "claude-haiku-5-5";

impl Config {
    /// Diagrams are drawn by Claude Haiku (your API key).
    pub fn haiku_diagrams(&self) -> bool {
        self.diagram_model == HAIKU
    }

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

    /// A diagram can be drawn: the chosen model is ready, or (Haiku without
    /// a key) a diagram model on this computer is.
    pub fn can_draw(&self) -> bool {
        self.diagram_model_installed() || (self.haiku_diagrams() && self.local_diagram_model().is_some())
    }

    /// What to do so that diagrams can be drawn, for the pill.
    pub fn diagram_setup(&self) -> &'static str {
        if self.haiku_diagrams() { "Diagrams: add your API key in Settings" } else { "Diagrams: get the model in Settings" }
    }

    /// The first diagram model on this computer: what draws when Haiku can't.
    pub fn local_diagram_model(&self) -> Option<Config> {
        MODELS
            .iter()
            .map(|m| Config { diagram_model: m.id.into(), ..self.clone() })
            .find(Config::diagram_model_installed)
    }

    /// Diagrams can be drawn: their model is here (downloaded and complete),
    /// or for Claude Haiku, there's an API key.
    pub fn diagram_model_installed(&self) -> bool {
        if self.haiku_diagrams() {
            return self.anthropic_key.is_some();
        }
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

/// "a sequence diagram of …" without its "a" (after "visualize" only: at the
/// start of a dictation, "A flowchart of the deploy is in the doc" is prose).
fn strip_article(text: &str) -> &str {
    let lower = text.to_lowercase();
    ["a ", "an ", "the "].iter().find(|a| lower.starts_with(*a)).map_or(text, |a| &text[a.len()..])
}

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
    let visualize = m.name("visualize").is_some();
    // "Visualize a sequence diagram of …": the kind said after it.
    if visualize
        && kind.is_none()
        && let Some(inner) = request(strip_article(about)).filter(|r| !r.visualize)
    {
        return Some(Request { visualize, ..inner });
    }
    Some(Request { kind, visualize, about: about.to_string() })
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
    /// Why the model in the settings didn't draw it, when another did
    /// (Claude couldn't be reached, so a model on this computer drew it).
    pub fallback: Option<String>,
    /// What the model wrote that isn't in the drawing, and why: said (in the
    /// terminal, in the log), never dropped quietly.
    pub left_out: Vec<String>,
}

impl Diagram {
    pub fn new(kind: Kind, mermaid: String) -> Diagram {
        Diagram { kind, mermaid, fallback: None, left_out: vec![] }
    }

    /// A flowchart laid out the way the speaker asked for: left to right when
    /// they said so. Their words decide, not whether the model caught them
    /// (the local one draws top-down whatever is said).
    pub(crate) fn laid_out_for(mut self, request: &Request) -> Diagram {
        if self.kind == Kind::Flowchart
            && left_to_right(&request.about)
            && let Some(rest) = self.mermaid.strip_prefix("flowchart TD\n")
        {
            self.mermaid = format!("flowchart LR\n{rest}");
        }
        self
    }
}

/// "Left to right", "horizontal": the flowchart is asked to go across.
static ACROSS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:left[\s-]+to[\s-]+right|horizontal(?:ly)?)\b").unwrap());

/// The dictation asks for a flowchart that goes from left to right.
fn left_to_right(about: &str) -> bool {
    ACROSS.is_match(about)
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

/// A flowchart node's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Step,
    Decision,
    /// Where it starts or ends.
    Terminal,
}

/// Write a flowchart: the nodes still in use (all of them when there are no
/// edges), numbered in their order, with quoted labels. None when fewer than
/// two are left.
pub(crate) fn flowchart(direction: &str, labels: &[(String, Shape)], edges: &[(usize, Option<String>, usize)]) -> Option<String> {
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
    for (a, label, b) in edges {
        match label.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            // (Labels never hold a |; quotes here would be drawn.)
            Some(l) => out += &format!("    {} -->|{l}| {}\n", id(*a), id(*b)),
            None => out += &format!("    {} --> {}\n", id(*a), id(*b)),
        }
    }
    Some(out)
}

/// The most steps a flowchart is drawn with.
const MOST_STEPS: usize = 30;

/// The flowchart the model meant, and what of its answer isn't in it. Each
/// step once: a label written again is the same step (a decision and a step
/// can share their words: "Add milk?" then "Add milk"), and an id given a
/// second label keeps its first. A small model that doesn't know when to stop
/// goes round: an arrow drawn again or a step pointing at itself is skipped,
/// and the drawing goes on with what follows.
fn tidy_flowchart(lines: &[&str], direction: &str) -> Option<(String, Vec<String>)> {
    let mut labels: Vec<(String, Shape)> = vec![]; // by node number
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_label: HashMap<(String, bool), usize> = HashMap::new();
    let mut edges: Vec<(usize, Option<String>, usize)> = vec![];
    let mut left_out: Vec<String> = vec![];
    // The node a line names: known by its id, else by its label, else new.
    let mut node = |text: &str, labels: &mut Vec<(String, Shape)>, left_out: &mut Vec<String>| -> Option<usize> {
        let c = NODE.captures(text.trim())?;
        let id = c[1].to_string();
        let given = c.get(2).map(|m| (m.as_str(), Shape::Terminal))
            .or(c.get(3).map(|m| (m.as_str(), Shape::Step)))
            .or(c.get(4).map(|m| (m.as_str(), Shape::Decision)));
        if let Some(&n) = by_id.get(&id) {
            match given.filter(|(l, _)| key(l) != key(&labels[n].0)) {
                // Pointed at before it was named ("A --> B", then "B[Boil water]"): its label.
                Some((label, shape)) if labels[n].0 == id => {
                    by_label.entry((key(label), shape == Shape::Decision)).or_insert(n);
                    labels[n] = (label.trim().to_string(), shape);
                }
                // A new label on a named id is the model losing track: keep the first.
                Some((label, _)) => left_out.push(format!("\"{}\": {id} is already \"{}\"", label.trim(), labels[n].0)),
                None => {}
            }
            return Some(n);
        }
        let (label, shape) = given.map_or((id.clone(), Shape::Step), |(l, s)| (l.trim().to_string(), s));
        let n = *by_label.entry((key(&label), shape == Shape::Decision)).or_insert_with(|| {
            labels.push((label, shape));
            labels.len() - 1
        });
        by_id.insert(id, n);
        Some(n)
    };
    let lines: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let before = labels.len();
        let arrow = EDGE.captures(line);
        let ends = match &arrow {
            Some(c) => node(&c[1], &mut labels, &mut left_out).zip(node(&c[3], &mut labels, &mut left_out)),
            None => node(line, &mut labels, &mut left_out).map(|n| (n, n)),
        };
        if labels.len() > MOST_STEPS {
            labels.truncate(before);
            left_out.push(format!("what followed ({} lines): only {MOST_STEPS} steps are drawn", lines.len() - i));
            break;
        }
        let Some((from, to)) = ends else {
            left_out.push(format!("\"{line}\": not a step or an arrow"));
            continue;
        };
        let Some(c) = arrow else { continue }; // a step on a line of its own
        if from == to {
            left_out.push(format!("\"{line}\": \"{}\" pointing at itself", labels[from].0));
            continue;
        }
        // The same arrow again (or again without its label) is the model going round.
        let label = c.get(2).map(|m| m.as_str().trim().to_string()).filter(|l| !l.is_empty());
        let again = |(a, l, b): &(usize, Option<String>, usize)| {
            (*a, *b) == (from, to) && (label.is_none() || l.as_deref().map(key) == label.as_deref().map(key))
        };
        if !edges.iter().any(again) {
            edges.push((from, label, to));
        }
    }
    let mermaid = flowchart(direction, &labels, &edges)?;
    if !edges.is_empty() {
        let used: HashSet<usize> = edges.iter().flat_map(|&(a, _, b)| [a, b]).collect();
        let loose = labels.iter().enumerate().filter(|(n, _)| !used.contains(n));
        left_out.extend(loose.map(|(_, (label, _))| format!("\"{label}\": no arrow to or from it")));
    }
    Some((mermaid, left_out))
}

/// The mind map, up to the first topic said again under the same parent
/// (where it goes round), and what was left out there. The same words under
/// another parent are another topic: "Pros" and "Cons" under each laptop.
fn tidy_mindmap(root: &str, lines: &[&str]) -> Option<(String, Vec<String>)> {
    let mut seen: HashSet<Vec<String>> = HashSet::new(); // each topic by its path from the root
    let mut path: Vec<String> = vec![]; // the branch being written
    let mut out = format!("mindmap\n  root(({}))\n", root.trim());
    let mut left_out = vec![];
    let mut count = 0;
    let lines: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let text = line.trim();
        let indent = line.len() - line.trim_start().len();
        // A branch can only go one level deeper than the line before.
        let depth = ((indent.max(4) - 4) / 2 + 1).min(path.len() + 1);
        path.truncate(depth - 1);
        path.push(key(text));
        if key(text) == key(root) || !seen.insert(path.clone()) {
            let after = match lines.len() - i - 1 {
                0 => String::new(),
                1 => ", and the line after it".to_string(),
                n => format!(", and the {n} lines after it"),
            };
            left_out.push(format!("\"{text}\" said again{after}"));
            break;
        }
        let text = NUMBERED.replace(text, "");
        out += &format!("{}{}\n", "  ".repeat(depth + 1), text.trim_end_matches(':'));
        count += 1;
    }
    (count >= 1).then_some((out, left_out))
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
        return tidy_flowchart(&rest, direction)
            .map(|(mermaid, left_out)| Diagram { left_out, ..Diagram::new(Kind::Flowchart, mermaid) });
    }
    if head == "mindmap" {
        let (root, branches) = rest.split_first()?;
        let root = root.trim().strip_prefix("root((")?.strip_suffix("))")?;
        return tidy_mindmap(root, branches)
            .map(|(mermaid, left_out)| Diagram { left_out, ..Diagram::new(Kind::Mindmap, mermaid) });
    }
    if head == "sequenceDiagram" {
        return tidy_sequence(&rest).map(|mermaid| Diagram::new(Kind::Sequence, mermaid));
    }
    None
}

impl Localflow {
    /// Draw what a dictation asked for, with the diagram model from the
    /// settings. The cleanup model, loaded while you spoke, makes room for it.
    /// With Claude Haiku, a diagram model on this computer draws when the API
    /// can't be reached.
    pub fn diagram(&self, request: &Request) -> Result<Diagram> {
        if self.cfg.haiku_diagrams() {
            let haiku = self.haiku_diagram(request);
            return match (haiku, self.cfg.local_diagram_model()) {
                (Err(e), Some(local)) => {
                    *self.engine.lock().unwrap() = None; // room for the local one
                    let drawn = Localflow::new(local).diagram(request).map_err(|l| Error(format!("{e}; on this computer: {l}")))?;
                    Ok(Diagram { fallback: Some(e.0), ..drawn })
                }
                (result, _) => result,
            };
        }
        if !self.cfg.diagrams() {
            return err("diagrams are off (Settings > Visualize, or LOCALFLOW_DIAGRAM_MODEL)");
        }
        if !self.cfg.diagram_model_installed() {
            return err(match self.cfg.diagram_download() {
                Some(_) => "the diagram model isn't downloaded yet".to_string(),
                None => format!("the diagram model's file wasn't found: {}", self.cfg.diagram_model),
            });
        }
        *self.engine.lock().unwrap() = None;
        let mut engine = self.diagram_engine.lock().unwrap();
        if engine.is_none() {
            *engine = Some(load_model(&self.cfg.diagram_model_file(), self.cfg.cleanup_gpu)?);
        }
        // Cut off at the limit is fine: tidy draws what the circles repeat once.
        let (answer, _) =
            engine.as_mut().unwrap().complete_with(&prompt(request), Some(&grammar(request.kind)), 900).map_err(Error)?;
        tidy(&answer).map(|d| d.laid_out_for(request)).ok_or_else(|| Error("couldn't draw that".into()))
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
        assert_eq!(req("Visualize a sequence diagram of the app asking the server"),
                   Some((Some(Kind::Sequence), "the app asking the server".into())));
        assert!(request("visualize create a sequence diagram: the app asks the server").unwrap().visualize);
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
                          "The flowchart is wrong, fix the second step please.", "Let's visualize success",
                          "A flowchart of the deploy is in the doc"] {
            assert_eq!(req(dictation), None, "{dictation}");
        }
    }

    #[test]
    fn grammar_per_kind() {
        assert!(grammar(None).starts_with("root ::= flowchart | mindmap\n"));
        assert!(grammar(Some(Kind::Mindmap)).starts_with("root ::= mindmap\n"));
    }

    #[test]
    fn flowchart_going_round_is_drawn_once() {
        let answer = "flowchart TD\n    A([Sign in]) --> B{Password right?}\n    B -->|Yes| C[Show dashboard]\n    \
                      B -->|No| D[Show error]\n    D --> A\n    C --> E[Show dashboard]\n    E --> F[Done]\n    \
                      A --> B\n    B --> C\n";
        let d = tidy(answer).unwrap();
        assert_eq!(d.kind, Kind::Flowchart);
        // The step written again is the same step, the arrows drawn again are
        // skipped, and "Done" after them is still drawn.
        assert_eq!(d.mermaid, "flowchart TD\n    N1([\"Sign in\"])\n    N2{\"Password right?\"}\n    \
                               N3[\"Show dashboard\"]\n    N4[\"Show error\"]\n    N5[\"Done\"]\n    N1 --> N2\n    \
                               N2 -->|Yes| N3\n    N2 -->|No| N4\n    N4 --> N1\n    N3 --> N5\n");
        assert_eq!(d.left_out, ["\"C --> E[Show dashboard]\": \"Show dashboard\" pointing at itself"]);
        // An id given a new label keeps its first; the edge back to it is the loop.
        let answer = "flowchart LR\n    A[Open PR] --> B[Run tests]\n    B --> C[Merge]\n    C --> A[Something else]\n    \
                      A --> B\n";
        let d = tidy(answer).unwrap();
        assert_eq!(d.mermaid, "flowchart LR\n    N1[\"Open PR\"]\n    N2[\"Run tests\"]\n    \
                               N3[\"Merge\"]\n    N1 --> N2\n    N2 --> N3\n    N3 --> N1\n");
        assert_eq!(d.left_out, ["\"Something else\": A is already \"Open PR\""]);
        assert_eq!(tidy("flowchart TD\n    A[Only one]\n"), None);
    }

    #[test]
    fn decision_and_step_with_the_same_words() {
        // What Qwen3.5 2B drew for "making tea: boil water, add the tea bag,
        // wait three minutes, remove the bag, add milk if you like". The
        // decision "Add tea bag?" and the step "Add tea bag" were once one
        // node, the arrow between them a loop, and the drawing ended there.
        let answer = "flowchart TD\n    A[Start] --> B[Boil water]\n    B --> C{Add tea bag?}\n    \
                      C -->|Yes| D[Add tea bag]\n    D --> E[Wait three minutes]\n    E --> F[Remove the bag]\n    \
                      F --> G{Add milk?}\n    G -->|Yes| H[Add milk]\n    G -->|No| I[Done]\n    H --> I\n";
        let d = tidy(answer).unwrap();
        assert_eq!(d.mermaid, "flowchart TD\n    N1[\"Start\"]\n    N2[\"Boil water\"]\n    N3{\"Add tea bag?\"}\n    \
                               N4[\"Add tea bag\"]\n    N5[\"Wait three minutes\"]\n    N6[\"Remove the bag\"]\n    \
                               N7{\"Add milk?\"}\n    N8[\"Add milk\"]\n    N9[\"Done\"]\n    N1 --> N2\n    N2 --> N3\n    \
                               N3 -->|Yes| N4\n    N4 --> N5\n    N5 --> N6\n    N6 --> N7\n    N7 -->|Yes| N8\n    \
                               N7 -->|No| N9\n    N8 --> N9\n");
        assert_eq!(d.left_out, [] as [&str; 0]);
        // A step pointed at before it's named gets its label, not its id.
        let d = tidy("flowchart TD\n    A[Boil water] --> B\n    B{Milk?} -->|Yes| C[Add milk]\n").unwrap();
        assert_eq!(d.mermaid, "flowchart TD\n    N1[\"Boil water\"]\n    N2{\"Milk?\"}\n    N3[\"Add milk\"]\n    \
                               N1 --> N2\n    N2 -->|Yes| N3\n");
        assert_eq!(d.left_out, [] as [&str; 0]);
        // The same step under a new id is still one step (punctuation and case aside).
        let d = tidy("flowchart TD\n    A[Boil water] --> B[Add milk]\n    C[boil water.] --> D[Stir]\n").unwrap();
        assert_eq!(d.mermaid, "flowchart TD\n    N1[\"Boil water\"]\n    N2[\"Add milk\"]\n    N3[\"Stir\"]\n    \
                               N1 --> N2\n    N1 --> N3\n");
    }

    #[test]
    fn nothing_is_dropped_quietly() {
        // A step pointing at itself, a line that isn't one, a step with no
        // arrows: each skipped and said, and the lines after them drawn.
        let answer = "flowchart TD\n    A[Wake up] --> A\n    A --> B[Shower]\n    B ==> C\n    Z[Nap]\n    \
                      B -->|Late| D[Run]\n    B -->|Early| D\n    B --> D\n    B -->|late| D\n";
        let d = tidy(answer).unwrap();
        assert_eq!(d.mermaid, "flowchart TD\n    N1[\"Wake up\"]\n    N2[\"Shower\"]\n    N3[\"Run\"]\n    \
                               N1 --> N2\n    N2 -->|Late| N3\n    N2 -->|Early| N3\n");
        assert_eq!(d.left_out, ["\"A[Wake up] --> A\": \"Wake up\" pointing at itself",
                                "\"B ==> C\": not a step or an arrow", "\"Nap\": no arrow to or from it"]);
        // More steps than are drawn: the rest is said too.
        let long: String = (0..40u8).map(|i| format!("    A{} --> {}{}\n", i % 10, (b'B' + i / 10) as char, i % 10)).collect();
        let d = tidy(&format!("flowchart TD\n{long}")).unwrap();
        assert_eq!(d.mermaid.matches(" --> ").count(), 20);
        assert_eq!(d.left_out, ["what followed (20 lines): only 30 steps are drawn"]);
    }

    #[test]
    fn left_to_right_when_said() {
        let ask = |said: &str| request(said).unwrap();
        let drawn = |said: &str, answer: &str| tidy(answer).unwrap().laid_out_for(&ask(said)).mermaid;
        let top_down = "flowchart TD\n    A[Write code] --> B[Open a pull request]\n    B --> C[Merge]\n";
        // The model drew it top-down; the speaker said left to right.
        for said in ["Visualize a flowchart left to right: write code, open a pull request, merge",
                     "Draw a flowchart of the release, from left-to-right: write code, open a pull request, merge",
                     "Visualize the release as a horizontal flowchart: write code, open a pull request, merge",
                     "Flowchart: write code, open a pull request, merge, laid out horizontally"] {
            assert!(drawn(said, top_down).starts_with("flowchart LR\n    N1[\"Write code\"]\n"), "{said}");
        }
        // Not said: as the model drew it, either way.
        let plain = "Visualize the release: write code, open a pull request, then merge right away";
        assert!(drawn(plain, top_down).starts_with("flowchart TD\n"));
        assert!(drawn(plain, &top_down.replace(" TD", " LR")).starts_with("flowchart LR\n"));
        // Only a flowchart has a direction.
        let mindmap = "mindmap\n  root((Layout))\n    Left to right\n    Top down\n";
        assert_eq!(drawn("Mind map of layouts: left to right, and top down", mindmap), mindmap);
    }

    #[test]
    fn mindmap_and_sequence() {
        let d = tidy("mindmap\n  root((Japan))\n    1. Tokyo\n      Shibuya\n    Kyoto\n        Temples\n    Osaka\n    \
                      Kyoto\n    Osaka\n").unwrap();
        assert_eq!(d.mermaid, "mindmap\n  root((Japan))\n    Tokyo\n      Shibuya\n    Kyoto\n      Temples\n    Osaka\n");
        assert_eq!(d.left_out, ["\"Kyoto\" said again, and the line after it"]);
        // The same words under another parent are another topic, not the map going round.
        let laptops = "mindmap\n  root((Laptops))\n    MacBook\n      Pros\n        Battery\n      Cons\n        Price\n    \
                       ThinkPad\n      Pros\n        Keyboard\n      Cons\n        Screen\n";
        let d = tidy(laptops).unwrap();
        assert_eq!((d.mermaid.as_str(), d.left_out.len()), (laptops, 0));
        let d = tidy("sequenceDiagram\n    App->>API: Get user\n    API-->>App: User\n    App->>API: Get user\n").unwrap();
        assert_eq!(d.mermaid, "sequenceDiagram\n    App->>API: Get user\n    API-->>App: User\n");
        assert_eq!(tidy("Here is your diagram"), None);
    }
}
