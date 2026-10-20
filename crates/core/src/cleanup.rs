//! The two checkpoints: a few-shot draft, then automatic checks whose
//! problems (if any) a fresh call repairs. Plus the sanity check that falls
//! back to the raw transcript, and Markdown headings small models miss.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;
use serde_json::{Value, json};

use crate::prompts::{ASIDE, ASIDE_EXAMPLES, EXAMPLES, QUALIFIERS, REVIEW_SYSTEM};
use crate::vocab::Hint;
use crate::{Localflow, Msg, Result};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

static DICTATION_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"</?(?:dictation|quiet)>"));
/// What was muttered (rule 12): meant to go, so the checks don't miss it.
static QUIET: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<quiet>.*?</quiet>"));
static FILE_REF: LazyLock<Regex> = LazyLock::new(|| re(r"@(\S+)"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[a-z']+"));
static JSON_OBJECT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\{.*\}"));
// A retraction left in place: "... pretty good, actually this is not pretty good, this is awesome".
static RETRACTION: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:actually|wait|no|sorry)\b[^.!?\n]{0,25}?\bnot ((?:[a-z']+ ?){1,4}?)\b[,.]"));
static SECTION_LEADIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(?:on|for|about|regarding) (?:the )?([\w'&/ -]{1,30}?)(?: side| front)?:$"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][\w'&/ -]{0,40}$"));

/// The cleaned text, and how it got there (for logs and `--eval`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cleaned {
    /// After the first checkpoint.
    pub draft: String,
    /// What to paste: the draft, its repair, or the raw transcript.
    pub text: String,
    pub verdict: String,
    pub ms_draft: u128,
    pub ms_review: u128,
}

/// A piece of cleaned text: words to paste, or a project file the speaker named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    File(String),
}

/// The dictation, plus the project files it may refer to (rule 9).
pub fn user_turn(text: &str, files: &[impl AsRef<str>]) -> String {
    let mut turn = format!("<dictation>{text}</dictation>");
    if !files.is_empty() {
        let listed: Vec<&str> = files.iter().map(AsRef::as_ref).collect();
        turn += &format!("\n<files>\n{}\n</files>", listed.join("\n"));
    }
    turn
}

/// The part of every draft request that doesn't depend on the dictation.
/// `asides`: with rule 12's examples.
pub fn prefix_messages(system: &str, asides: bool) -> Vec<Msg> {
    let mut messages = vec![Msg::system(system)];
    let asides = ASIDE_EXAMPLES.iter().filter(|_| asides).map(|(raw, cleaned)| (*raw, *cleaned, &[] as &[&str]));
    for (raw, cleaned, files) in EXAMPLES.iter().copied().chain(asides) {
        messages.push(Msg::user(user_turn(raw, files)));
        messages.push(Msg::assistant(cleaned));
    }
    messages
}

/// Cleanup never adds much. A much longer reply means the model answered or
/// chatted instead of cleaning. File paths don't count.
pub fn plausible(out: &str, text: &str, files: &[impl AsRef<str>]) -> bool {
    let mut paths: Vec<&str> = files.iter().map(AsRef::as_ref).collect();
    paths.sort_by_key(|p| Reverse(p.chars().count()));
    let mut out = out.to_string();
    for path in paths {
        out = out.replace(&format!("@{path}"), "@");
    }
    !out.trim().is_empty() && out.chars().count() as f64 <= text.chars().count() as f64 * 1.5 + 40.0
}

/// Split cleaned text into text and file parts. Only paths from `files`
/// count; anything else after an @ stays text.
pub fn file_refs(out: &str, files: &[impl AsRef<str>]) -> Vec<Part> {
    let known: HashSet<&str> = files.iter().map(AsRef::as_ref).collect();
    let (mut parts, mut pos) = (vec![], 0);
    for m in FILE_REF.captures_iter(out) {
        let start = m.get(0).unwrap().start();
        let mut path = &m[1];
        while !path.is_empty() && !known.contains(path) && path.ends_with(['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"']) {
            path = &path[..path.len() - 1]; // sentence punctuation after the path
        }
        if known.contains(path) {
            parts.push(Part::Text(out[pos..start].to_string()));
            parts.push(Part::File(path.to_string()));
            pos = start + 1 + path.len();
        }
    }
    parts.push(Part::Text(out[pos..].to_string()));
    parts.retain(|p| !matches!(p, Part::Text(t) | Part::File(t) if t.is_empty()));
    parts
}

fn words_in(t: &str) -> HashSet<String> {
    WORD.find_iter(&t.to_lowercase()).map(|m| m.as_str().to_string()).collect()
}

fn has_phrase(t: &str, phrase: &str) -> bool {
    re(&format!(r"(?i)\b{}\b", regex::escape(phrase))).is_match(t)
}

/// Concrete, checkable mistakes in the draft, each phrased as an instruction
/// for the repair pass.
pub fn find_problems(text: &str, out: &str, hints: &[Hint]) -> Vec<String> {
    let mut problems = vec![];
    let text = &QUIET.replace_all(text, " ");
    let (raw_words, out_words) = (words_in(text), words_in(out));
    for q in QUALIFIERS {
        if raw_words.contains(*q) && !out_words.contains(*q) {
            problems.push(format!("The speaker said \"{q}\" but it was dropped. Put it back where it belongs, \
                                   unless it was part of a correction the speaker took back."));
        }
    }
    let low = out.to_lowercase();
    for m in RETRACTION.captures_iter(&low) {
        let whole = m.get(0).unwrap();
        let said = m[1].trim();
        if !said.is_empty() && low.find(said).is_none_or(|p| p < whole.start()) {
            problems.push(format!("The speaker took back \"{said}\" (\"{}\"). Remove the retracted version and \
                                   the retraction itself, keeping only what they said instead.",
                                  whole.as_str().trim_matches([' ', ',', '.'])));
        }
    }
    for (term, heard) in hints {
        for h in heard {
            if h.to_lowercase() == term.to_lowercase() {
                continue;
            }
            // Flag when the misheard form survived, or the right term never appeared.
            if has_phrase(text, h) && (has_phrase(out, h) || !out.to_lowercase().contains(&term.to_lowercase())) {
                problems.push(format!("The speaker's \"{h}\" means \"{term}\". Use that exact spelling."));
            }
        }
    }
    problems
}

/// Small models often write section titles as bare lines ("Sales") or
/// lead-ins ("On hiring:"). When there are two or more (on their own line,
/// after a blank line, followed by text), make them "## " headings.
pub fn markdown_headings(out: &str) -> String {
    let mut lines: Vec<String> = out.split('\n').map(String::from).collect();
    let starts_section = |lines: &[String], i: usize| {
        (i == 0 || lines[i - 1].trim().is_empty()) && i + 1 < lines.len() && !lines[i + 1].trim().is_empty()
    };
    let leadins: Vec<usize> = (0..lines.len())
        .filter(|&i| SECTION_LEADIN.is_match(lines[i].trim()) && starts_section(&lines, i))
        .collect();
    if leadins.len() >= 2 {
        for &i in &leadins {
            let title = SECTION_LEADIN.captures(lines[i].trim()).unwrap()[1].trim().to_string();
            let mut chars = title.chars();
            let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
            lines[i] = first + chars.as_str();
        }
    }
    let titles: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            TITLE.is_match(lines[i].trim()) && lines[i].split_whitespace().count() <= 4 && starts_section(&lines, i)
        })
        .collect();
    if titles.len() < 2 {
        return out.to_string();
    }
    for i in titles {
        lines[i] = format!("## {}", lines[i].trim());
    }
    lines.join("\n")
}

impl Localflow {
    /// Checkpoint 1: few-shot cleanup and formatting.
    fn draft(&self, text: &str, model: &str, system: &str, files: &[impl AsRef<str>]) -> Result<String> {
        let mut messages = prefix_messages(system, self.cfg.asides);
        messages.push(Msg::user(user_turn(text, files)));
        let budget = (text.chars().count() as f64 * 1.5) as usize
            + 64
            + files.iter().map(|f| f.as_ref().chars().count()).sum::<usize>();
        let out = self.chat(model, &messages, budget, None)?;
        Ok(DICTATION_TAG.replace_all(&out, "").trim().to_string())
    }

    /// Checkpoint 2: automatic checks, then a fresh call (only the rules, raw
    /// text, draft and problem list) that repairs what the checks found.
    /// Returns the verdict and the repaired text, if any.
    fn review(&self, text: &str, first: &str, model: &str, system: &str) -> Result<(String, Option<String>)> {
        let hints = self.misheard_hints();
        let problems = find_problems(text, first, &hints);
        if problems.is_empty() {
            return Ok(("ok".into(), None));
        }
        if model == "off" {
            return Ok((format!("{} problems, repair off", problems.len()), None));
        }
        let listed: Vec<String> = problems.iter().map(|p| format!("- {p}")).collect();
        let prompt = format!("RULES:\n{system}\n\nRAW:\n{text}\n\nDRAFT:\n{first}\n\nPROBLEMS:\n{}", listed.join("\n"));
        let format = json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]});
        let out = self.chat(model, &[Msg::system(REVIEW_SYSTEM), Msg::user(prompt)],
                            text.chars().count() * 2 + 128, Some(&format))?;
        // Ollama enforces the JSON format; hosted models may wrap it in a code block.
        let found = JSON_OBJECT.find(&out).map_or(out.as_str(), |m| m.as_str());
        let fixed = match serde_json::from_str::<Value>(found) {
            Ok(Value::Object(answer)) => match answer.get("text") {
                Some(Value::String(s)) => s.trim().to_string(),
                None | Some(Value::Null) | Some(Value::Bool(false)) => String::new(),
                Some(_) => return Ok(("invalid".into(), None)),
            },
            _ => return Ok(("invalid".into(), None)),
        };
        // Accept the repair only if it clears problems without adding new ones;
        // small models sometimes "fix" by dropping unrelated words.
        let remaining = if fixed.is_empty() { problems.clone() } else { find_problems(text, &fixed, &hints) };
        let was: HashSet<&String> = problems.iter().collect();
        if !fixed.is_empty() && fixed != first && remaining.len() < problems.len()
            && remaining.iter().all(|p| was.contains(p)) {
            return Ok((format!("fix ({}->{})", problems.len(), remaining.len()), Some(fixed)));
        }
        Ok((format!("kept ({})", problems.len()), None))
    }

    /// Both checkpoints, with the default models. `files`: paths in the project
    /// you're dictating into, which the text may refer to; the cleanup writes
    /// those references as @path.
    pub fn clean_detailed(&self, text: &str, files: &[impl AsRef<str>]) -> Result<Cleaned> {
        let (model, review_model) = (self.cfg.model.clone(), self.cfg.review_model.clone());
        self.clean_with(text, &model, &review_model, files)
    }

    pub fn clean_with(&self, text: &str, model: &str, review_model: &str, files: &[impl AsRef<str>]) -> Result<Cleaned> {
        let text = text.trim();
        let mut result = Cleaned { draft: text.into(), text: text.into(), verdict: "skipped".into(), ms_draft: 0,
                                   ms_review: 0 };
        if text.is_empty() {
            return Ok(result);
        }
        let system = self.rules();
        let start = Instant::now();
        let first = self.draft(text, model, &system, files)?;
        result.ms_draft = start.elapsed().as_millis();
        if self.cfg.asides && first == ASIDE {
            // Nothing meant for the text.
            (result.draft, result.text, result.verdict) = (String::new(), String::new(), "aside".into());
            return Ok(result);
        }
        if !plausible(&first, text, files) {
            return Ok(result); // keep what the speaker said
        }
        result.draft = markdown_headings(&first);
        result.text = result.draft.clone();
        let start = Instant::now();
        let (verdict, fixed) = self.review(text, &first, review_model, &system)
            .unwrap_or_else(|e| (format!("error: {e}"), None)); // the draft is good enough to use
        result.ms_review = start.elapsed().as_millis();
        result.verdict = verdict;
        match fixed {
            Some(fixed) if plausible(&fixed, text, files) => result.text = markdown_headings(&fixed),
            Some(_) => result.verdict = "fix rejected".into(),
            None => {}
        }
        Ok(result)
    }

    /// The cleaned transcript, or the original if anything looks off. An error
    /// means the model couldn't be reached: paste the raw transcript then.
    pub fn clean(&self, text: &str, files: &[impl AsRef<str>]) -> Result<String> {
        Ok(self.clean_detailed(text, files)?.text)
    }
}
