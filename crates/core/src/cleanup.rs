//! The cleanup: localflow's own model's answer, the sanity check that falls
//! back to the raw transcript, and Markdown headings small models miss.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;

use crate::local::ASIDE;
use crate::{Localflow, Result};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

static DICTATION_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"</?(?:dictation|quiet)>"));
static FILE_REF: LazyLock<Regex> = LazyLock::new(|| re(r"@(\S+)"));
static SECTION_LEADIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(?:on|for|about|regarding) (?:the )?([\w'&/ -]{1,30}?)(?: side| front)?:$"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][\w'&/ -]{0,40}$"));

/// The cleaned text, and how it got there (for logs and `--eval`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cleaned {
    /// What to paste: the model's answer, or the raw transcript.
    pub text: String,
    /// "ok", "aside" (nothing to paste), "implausible" (the raw transcript)
    /// or "skipped" (nothing was said).
    pub verdict: String,
    pub ms: u128,
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
    /// The cleanup, with how it went. `files`: paths in the project you're
    /// dictating into, which the text may refer to; the cleanup writes those
    /// references as @path.
    pub fn clean_detailed(&self, text: &str, files: &[impl AsRef<str>]) -> Result<Cleaned> {
        let text = text.trim();
        let mut result = Cleaned { text: text.into(), verdict: "skipped".into(), ms: 0 };
        if text.is_empty() {
            return Ok(result);
        }
        let budget = (text.chars().count() as f64 * 1.5) as usize
            + 64
            + files.iter().map(|f| f.as_ref().chars().count()).sum::<usize>();
        let start = Instant::now();
        let out = self.local_complete(&crate::local::cleanup_prompt(&self.vocabulary(), text, files), budget)?;
        let out = DICTATION_TAG.replace_all(&out, "").trim().to_string();
        result.ms = start.elapsed().as_millis();
        // localflow-clean leaves asides out (it was trained with rule 12).
        if out == ASIDE {
            // Nothing meant for the text.
            (result.text, result.verdict) = (String::new(), "aside".into());
        } else if !plausible(&out, text, files) {
            result.verdict = "implausible".into(); // keep what the speaker said
        } else {
            (result.text, result.verdict) = (markdown_headings(&out), "ok".into());
        }
        Ok(result)
    }

    /// The cleaned transcript, or the original if anything looks off. An error
    /// means the model couldn't run: paste the raw transcript then.
    pub fn clean(&self, text: &str, files: &[impl AsRef<str>]) -> Result<String> {
        Ok(self.clean_detailed(text, files)?.text)
    }
}
