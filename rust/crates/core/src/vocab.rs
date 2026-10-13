//! The speaker's vocabulary (words.txt, and what learning adds), and learning
//! from the fixes you make to a pasted dictation.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::difflib::{SequenceMatcher, Tag};
use crate::prompts::{COMMON, JUDGE_SYSTEM};
use crate::{Localflow, Msg, Result};

/// Names and terms that ship with localflow, before your own.
const WORDS: &str = include_str!("../../../../words.txt");

/// A term and the ways speech recognition mishears it.
pub type Hint = (String, Vec<String>);

static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)""#).unwrap());
static JSON_OBJECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\{.*\}").unwrap());

const PUNCT: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\'', '(', ')', '[', ']'];

fn entries(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().map(str::trim).filter(|w| !w.is_empty() && !w.starts_with('#')).map(String::from)
}

impl Localflow {
    pub fn vocabulary(&self) -> Vec<String> {
        let mut words: Vec<String> = entries(WORDS).collect();
        if let Ok(mine) = std::fs::read_to_string(self.cfg.user_words()) {
            words.extend(entries(&mine));
        }
        words
    }

    /// (term, [ways it gets misheard]) from words.txt lines like
    /// 'Pine Script: TradingView's language, often heard as "Py script"'.
    pub fn misheard_hints(&self) -> Vec<Hint> {
        self.vocabulary()
            .iter()
            .filter_map(|w| {
                let (term, hint) = w.split_once(':')?;
                let (_, heard) = hint.split_once("heard as")?;
                Some((term.trim().to_string(), QUOTED.captures_iter(heard).map(|c| c[1].to_string()).collect()))
            })
            .collect()
    }

    pub fn rules(&self) -> String {
        let mut system = crate::prompts::SYSTEM.to_string();
        let words = self.vocabulary();
        if !words.is_empty() {
            system += "\n\nSpeaker's vocabulary (use these exact spellings):\n";
            system += &words.iter().map(|w| format!("- {w}")).collect::<Vec<_>>().join("\n");
        }
        system
    }

    /// Of the changes you made to a pasted dictation, the ones that fix a
    /// misheard word, not a change of mind. Asks the cleanup model.
    pub fn mishearings(&self, pasted: &str, edited: &str, pairs: &[(String, String)], model: &str)
        -> Result<Vec<(String, String)>> {
        if pairs.is_empty() {
            return Ok(vec![]);
        }
        let listed: Vec<String> =
            pairs.iter().enumerate().map(|(i, (h, m))| format!("{}. \"{h}\" -> \"{m}\"", i + 1)).collect();
        let format = json!({"type": "object", "properties": {"learn": {"type": "array", "items": {"type": "integer"}}},
                            "required": ["learn"]});
        let out = self.chat(model, &[
            Msg::system(JUDGE_SYSTEM),
            Msg::user(format!("PASTED:\n{pasted}\n\nEDITED:\n{edited}\n\nCHANGES:\n{}", listed.join("\n"))),
        ], 200, Some(&format))?;
        let found = JSON_OBJECT.find(&out).map_or(out.as_str(), |m| m.as_str());
        let Ok(Value::Object(answer)) = serde_json::from_str::<Value>(found) else { return Ok(vec![]) };
        let mut seen = HashSet::new();
        Ok(answer.get("learn").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
            .iter()
            .filter_map(Value::as_i64)
            .filter(|&i| i >= 1 && i as usize <= pairs.len() && seen.insert(i))
            .map(|i| pairs[i as usize - 1].clone())
            .collect())
    }

    /// Add corrections to your vocabulary as 'Claude: often heard as "Claw"'
    /// (just 'GitHub' when only the spelling changed). Returns the pairs that
    /// were new.
    pub fn learn(&self, pairs: &[(String, String)]) -> std::io::Result<Vec<(String, String)>> {
        let path = self.cfg.user_words();
        let mut lines: Vec<String> = match std::fs::read_to_string(&path) {
            Ok(text) => text.lines().map(String::from).collect(),
            Err(_) => vec![
                "# Your names and terms, added by `dictate learn` (and by you). Same format as".into(),
                "# words.txt in localflow: one per line, optionally 'Term: often heard as \"x\"'.".into(),
            ],
        };
        let mut known: HashSet<(String, String)> = self.misheard_hints().into_iter()
            .flat_map(|(term, heard)| heard.into_iter().map(move |h| (term.to_lowercase(), h.to_lowercase())))
            .collect();
        let mut terms: HashSet<String> = self.vocabulary().iter()
            .map(|w| w.split(':').next().unwrap_or_default().trim().to_lowercase())
            .collect();
        let mut added = vec![];
        for (heard, meant) in pairs {
            let (h, m) = (heard.to_lowercase(), meant.to_lowercase());
            if h == m {
                if terms.insert(m) {
                    lines.push(meant.clone());
                    added.push((heard.clone(), meant.clone()));
                }
                continue;
            }
            if known.contains(&(m.clone(), h.clone())) {
                continue;
            }
            let existing = lines.iter().position(|line| {
                line.split(':').next().unwrap_or_default().trim().to_lowercase() == m && !line.starts_with('#')
            });
            match existing {
                Some(i) => {
                    let line = &lines[i];
                    let (term, hint) = line.split_once(':').unwrap_or((line, ""));
                    lines[i] = if hint.contains("heard as") {
                        format!("{line}, \"{heard}\"")
                    } else if hint.trim().is_empty() {
                        format!("{}: often heard as \"{heard}\"", term.trim())
                    } else {
                        format!("{line}, often heard as \"{heard}\"")
                    };
                }
                None => lines.push(format!("{meant}: often heard as \"{heard}\"")),
            }
            known.insert((m.clone(), h));
            terms.insert(m);
            added.push((heard.clone(), meant.clone()));
        }
        if !added.is_empty() {
            std::fs::create_dir_all(&self.cfg.config_dir)?;
            std::fs::write(&path, lines.join("\n") + "\n")?;
        }
        Ok(added)
    }
}

fn is_common(w: &str) -> bool {
    COMMON.split(' ').any(|c| c == w.to_lowercase())
}

/// What you fixed in a pasted dictation: [(heard, meant)] for each short
/// replacement (up to 3 words a side), e.g. [("Claw", "Claude")]. None when
/// `after` isn't an edited version of `before`.
pub fn corrections(before: &str, after: &str) -> Option<Vec<(String, String)>> {
    let tokens = |t: &str| -> Vec<String> {
        t.split_whitespace().map(|w| w.trim_matches(PUNCT)).filter(|w| !w.is_empty()).map(String::from).collect()
    };
    let (a, b) = (tokens(before), tokens(after));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let sm = SequenceMatcher::new(&a, &b);
    let same: usize = sm.matching_blocks().iter().map(|m| m.size).sum();
    if same < 2.max(b.len() / 2) {
        return None; // a different text, or rewritten rather than corrected
    }
    let mut pairs = vec![];
    for op in sm.opcodes() {
        let (heard, meant) = (a[op.i1..op.i2].join(" "), b[op.j1..op.j2].join(" "));
        if op.tag != Tag::Replace || op.i2 - op.i1 > 3 || op.j2 - op.j1 > 3 || heard == meant {
            continue;
        }
        if a[op.i1..op.i2].iter().chain(&b[op.j1..op.j2]).all(|w| is_common(w)) {
            continue;
        }
        pairs.push((heard, meant.replace(':', "")));
    }
    Some(pairs)
}

/// The part of `text` (a whole message or text box) that is the pasted
/// dictation, possibly edited; None if it isn't there.
pub fn edited_region(pasted: &str, text: &str) -> Option<String> {
    let a: Vec<&str> = pasted.split_whitespace().collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    if a.is_empty() || words.is_empty() {
        return None;
    }
    let key = |w: &&str| w.trim_matches(PUNCT).to_lowercase();
    let (ka, kw): (Vec<String>, Vec<String>) = (a.iter().map(key).collect(), words.iter().map(key).collect());
    let sm = SequenceMatcher::new(&ka, &kw);
    let blocks: Vec<_> = sm.matching_blocks().into_iter().filter(|m| m.size > 0).collect();
    if blocks.iter().map(|m| m.size).sum::<usize>() < 2.max(a.len() / 2) {
        return None;
    }
    let (first, last) = (blocks[0], blocks[blocks.len() - 1]);
    // Widen to the words around the matches that stand for unmatched pasted
    // words at either end, without crossing into another sentence.
    let stop = |w: &str| w.ends_with(['.', '!', '?']);
    let (mut start, mut need) = (first.b, first.a);
    while need > 0 && start > 0 && !stop(words[start - 1]) {
        (start, need) = (start - 1, need - 1);
    }
    let (mut end, mut need) = (last.b + last.size, a.len() - last.a - last.size);
    while need > 0 && end < words.len() {
        (end, need) = (end + 1, need - 1);
        if stop(a[a.len() - 1]) && stop(words[end - 1]) {
            break;
        }
    }
    Some(words[start..end].join(" "))
}
