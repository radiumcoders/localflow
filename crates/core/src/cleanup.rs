//! The cleanup: localflow's own model's answer (or Claude Haiku 5.5's), the
//! checks that fall back to the raw transcript, and Markdown headings small
//! models miss.

use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;

use serde_json::{Value, json};

use crate::difflib::{SequenceMatcher, Tag};
use crate::local::ASIDE;
use crate::prompts::{ASIDES_RULE, ASIDE_EXAMPLES, EXAMPLES, SYSTEM};
use crate::{Localflow, Result};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

static DICTATION_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"</?(?:dictation|quiet)>"));
static FILE_REF: LazyLock<Regex> = LazyLock::new(|| re(r"@(\S+)"));
static SECTION_LEADIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(?:on|for|about|regarding) (?:the )?([\w'&/ -]{1,30}?)(?: side| front)?:$"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][\w'&/ -]{0,40}$"));
static QUIET: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<quiet>.*?</quiet>"));
static MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\s*(?:\d+\.|[-*]|#+)\s"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[0-9]+(?:\.[0-9]+)?|[a-z0-9]+(?:'[a-z]+)?"));

/// The cleaned text, and how it got there (for logs and `--eval`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cleaned {
    /// What to paste: the model's answer, or the raw transcript.
    pub text: String,
    /// "ok", "aside" (nothing to paste), "implausible" or "unfaithful" (the
    /// raw transcript; with why), or "skipped" (nothing was said). After
    /// Claude Haiku couldn't clean up, it starts with why.
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

/// An answer of `<aside>` (nothing meant for the text) is believed only for a
/// short dictation, or one said entirely under the speaker's breath: a model
/// that drops half a minute of speech as an aside loses it for good.
pub fn believable_aside(text: &str) -> bool {
    WORD.find_iter(&QUIET.replace_all(&text.to_lowercase(), " ")).count() <= 10
}

/// The transcript as heard, to paste when the cleanup can't be trusted.
pub fn as_heard(text: &str) -> String {
    DICTATION_TAG.replace_all(text, "").split_whitespace().collect::<Vec<_>>().join(" ")
}

const NUMBERS: [&str; 11] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
/// Number words above ten, by value, which the cleanup may write as digits
/// ("thirty percent" -> 30%, "two thousand" -> 2000).
const BIG_NUMBERS: &[(&str, u64)] = &[
    ("eleven", 11), ("twelve", 12), ("thirteen", 13), ("fourteen", 14), ("fifteen", 15), ("sixteen", 16),
    ("seventeen", 17), ("eighteen", 18), ("nineteen", 19), ("twenty", 20), ("thirty", 30), ("forty", 40),
    ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90), ("dozen", 12),
];
const SCALES: &[(&str, u64)] = &[("hundred", 100), ("thousand", 1_000), ("million", 1_000_000), ("billion", 1_000_000_000)];

/// The numbers said in words, as digits: every run of number words, and each
/// part of it ("five thirty" is 5, 30 and 35, for "5:30").
fn spoken_numbers(said: &[String]) -> HashSet<String> {
    let value = |w: &str| -> Option<(u64, bool)> {
        // (value, is a scale); words() has already made zero to ten digits.
        match w.parse::<u64>() {
            Ok(n) if n <= 10 => Some((n, false)),
            _ => BIG_NUMBERS.iter().chain(SCALES).find(|(name, _)| *name == w).map(|&(_, n)| (n, SCALES.iter().any(|(s, _)| *s == w))),
        }
    };
    let mut found = HashSet::new();
    for start in 0..said.len() {
        let (mut total, mut current) = (0u64, 0u64);
        for word in &said[start..] {
            let Some((n, scale)) = value(word) else { break };
            match (scale, n) {
                (false, _) => current += n,
                (true, 100) => current = current.max(1) * 100,
                (true, _) => (total, current) = (total + current.max(1) * n, 0),
            }
            found.insert((total + current).to_string());
        }
    }
    found
}

/// Filler, counting words and correction phrases: the cleanup drops these.
const FILLER: &str = "um uh er ah hmm like so okay ok yeah you know mean actually wait no sorry oh well just basically \
                      right alright first second third fourth next then finally thing last";
/// Small words the cleanup may add, drop or swap for grammar.
const GRAMMAR: &str = "a an the and or but to of in on for at is are was be it this that these those with as i we you \
                       they he she my our your their make do does have has will would should can there here what";
/// Words near a dropped stretch that say it was replaced or set aside.
const CUES: &[&str] = &["wait", "no", "sorry", "actually", "mean", "scratch", "rather", "instead", "oh", "not", "hmm", "think"];

/// Lowercase words, with what the cleanup legitimately adds or removes (list
/// markers, @file references, muttered parts) left out, and number words as
/// digits.
fn words(text: &str) -> Vec<String> {
    let text = QUIET.replace_all(text, " ");
    let text = MARKER.replace_all(&text, " ");
    let text = FILE_REF.replace_all(&text, " ").to_lowercase().replace('%', " percent");
    WORD.find_iter(&text)
        .map(|m| NUMBERS.iter().position(|n| *n == m.as_str()).map_or(m.as_str().to_string(), |n| n.to_string()))
        .collect()
}

/// How alike two words are, 0 to 1 (difflib's ratio over letters).
fn alike(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let same: usize = SequenceMatcher::new(&a, &b).matching_blocks().iter().map(|m| m.size).sum();
    2.0 * same as f64 / (a.len() + b.len()).max(1) as f64
}

/// A word in the cleanup that the speaker said, near enough: the same word
/// respelled ("post gress" -> Postgres, "Claw" -> Claude), another form of it,
/// or a small word for grammar. A number has to be one they said (in words,
/// `numbers`, or digits).
fn explained(word: &str, said: &[String], numbers: &HashSet<String>) -> bool {
    if FILLER.split(' ').chain(GRAMMAR.split(' ')).any(|w| w == word) {
        return true;
    }
    if word.starts_with(|c: char| c.is_ascii_digit()) {
        return said.iter().any(|w| w == word) || numbers.contains(word);
    }
    let stem = |w: &str| w.chars().take(4).collect::<String>();
    (0..said.len()).any(|i| {
        alike(word, &said[i]) >= 0.6
            || (word.chars().count() >= 4 && stem(word) == stem(&said[i]))
            || (2..=3).any(|k| i + k <= said.len() && alike(word, &said[i..i + k].concat()) >= 0.75)
    })
}

/// What the cleanup got wrong, if anything: words the speaker never said
/// ("slash got shaders skill" -> "Claude Code", "root 2" -> "1.5"), or a
/// stretch of what they said left out, with no correction near it ("Do it
/// fast. I need it done fast." gone). Small models do both; the transcript as
/// heard is better than either.
pub fn unfaithful(text: &str, out: &str) -> Option<String> {
    let (said, wrote) = (words(text), words(out));
    let kept: HashSet<&String> = wrote.iter().collect();
    let numbers = spoken_numbers(&said);
    let small = |w: &String| FILLER.split(' ').chain(GRAMMAR.split(' ')).any(|f| f == w);
    for op in SequenceMatcher::new(&said, &wrote).opcodes() {
        if matches!(op.tag, Tag::Replace | Tag::Insert) {
            let new: Vec<&str> = wrote[op.j1..op.j2].iter().filter(|w| !explained(w, &said, &numbers)).map(String::as_str).collect();
            if !new.is_empty() {
                return Some(format!("added {:?}", new.join(" ")));
            }
        }
        if matches!(op.tag, Tag::Replace | Tag::Delete) {
            let lost: Vec<&str> =
                said[op.i1..op.i2].iter().filter(|w| !small(w) && !kept.contains(w)).map(String::as_str).collect();
            let near = &said[op.i1.saturating_sub(3)..(op.i2 + 3).min(said.len())];
            if lost.len() >= 3 && !near.iter().any(|w| CUES.contains(&w.as_str())) {
                return Some(format!("dropped {:?}", lost.join(" ")));
            }
        }
    }
    None
}

/// Claude Haiku's request: the rules, the examples as turns (cached: they're
/// the same every time), then the dictation.
pub(crate) fn haiku_body(rules: &str, text: &str, files: &[impl AsRef<str>], max_tokens: usize) -> Value {
    let asides = ASIDE_EXAMPLES.iter().map(|(raw, cleaned)| (*raw, *cleaned, &[] as &[&str]));
    let mut messages = vec![];
    for (raw, cleaned, files) in EXAMPLES.iter().copied().chain(asides) {
        messages.push(json!({"role": "user", "content": user_turn(raw, files)}));
        messages.push(json!({"role": "assistant", "content": cleaned}));
    }
    if let Some(last) = messages.last_mut() {
        let cleaned = last["content"].take();
        last["content"] = json!([{"type": "text", "text": cleaned, "cache_control": {"type": "ephemeral"}}]);
    }
    messages.push(json!({"role": "user", "content": user_turn(text, files)}));
    json!({
        "model": crate::diagram::HAIKU,
        "max_tokens": max_tokens,
        "thinking": {"type": "disabled"},
        "output_config": {"effort": "low"},
        "system": rules,
        "messages": messages,
    })
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
    /// The rules Claude Haiku cleans up with, and the speaker's vocabulary.
    fn rules(&self) -> String {
        let mut rules = format!("{SYSTEM}\n{ASIDES_RULE}");
        let words = self.vocabulary();
        if !words.is_empty() {
            rules += "\n\nSpeaker's vocabulary (use these exact spellings):\n";
            rules += &words.iter().map(|w| format!("- {w}")).collect::<Vec<_>>().join("\n");
        }
        rules
    }

    /// Claude Haiku 5.5's cleanup, with your API key.
    fn haiku_clean(&self, text: &str, files: &[impl AsRef<str>], budget: usize) -> Result<String> {
        let Some(key) = &self.cfg.anthropic_key else {
            return crate::err("no Anthropic API key (Settings > Cleanup)");
        };
        let body = haiku_body(&self.rules(), text, files, budget);
        let answer = crate::haiku::post(key, &body, std::time::Duration::from_secs(12))?;
        match answer["stop_reason"].as_str() {
            Some("max_tokens") => crate::err("Claude's cleanup was cut off"),
            Some("refusal") => crate::err("Claude declined"),
            _ => Ok(crate::haiku::answer_text(&answer)),
        }
    }

    /// The cleanup, with how it went. `files`: paths in the project you're
    /// dictating into, which the text may refer to; the cleanup writes those
    /// references as @path.
    pub fn clean_detailed(&self, text: &str, files: &[impl AsRef<str>]) -> Result<Cleaned> {
        let text = text.trim();
        let mut result = Cleaned { text: as_heard(text), verdict: "skipped".into(), ms: 0 };
        if text.is_empty() {
            return Ok(result);
        }
        let budget = (text.chars().count() as f64 * 1.5) as usize
            + 64
            + files.iter().map(|f| f.as_ref().chars().count()).sum::<usize>();
        let start = Instant::now();
        let mut note = String::new();
        let haiku = match self.cfg.cleanup_haiku {
            true => self.haiku_clean(text, files, budget).map_err(|e| note = format!("Claude Haiku: {e}; ")).ok(),
            false => None,
        };
        let out = match haiku {
            Some(out) => out,
            // Claude couldn't: the local model, when it's here.
            None if self.cfg.cleanup_haiku && !self.cfg.local_model_installed() => {
                result.verdict = format!("{note}pasted as heard");
                return Ok(result);
            }
            None => self.local_complete(&crate::local::cleanup_prompt(&self.vocabulary(), text, files), budget)?,
        };
        let by_haiku = self.cfg.cleanup_haiku && note.is_empty();
        let out = DICTATION_TAG.replace_all(&out, "").trim().to_string();
        result.ms = start.elapsed().as_millis();
        result.verdict = if out == ASIDE && believable_aside(text) {
            // Nothing meant for the text.
            result.text = String::new();
            "aside".into()
        } else if out == ASIDE || !plausible(&out, text, files) {
            "implausible".into() // keep what the speaker said
        } else if let Some(why) = (!by_haiku).then(|| unfaithful(text, &out)).flatten() {
            // Claude Haiku fixes misheard words the check can't tell from made-up ones.
            format!("unfaithful: {why}")
        } else {
            result.text = markdown_headings(&out);
            "ok".into()
        };
        result.verdict.insert_str(0, &note);
        Ok(result)
    }

    /// The cleaned transcript, or the original if anything looks off. An error
    /// means the model couldn't run: paste the raw transcript then.
    pub fn clean(&self, text: &str, files: &[impl AsRef<str>]) -> Result<String> {
        Ok(self.clean_detailed(text, files)?.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn made_up_words_are_caught() {
        // From real dictations that localflow-clean got wrong.
        let said = "create some cool random shaders with slash got shaders skill and use some random seeds";
        assert!(unfaithful(said, "Create some cool random shaders with Claude Code and use some random seeds.").is_some());
        let said = "a liquid of refractive index of root 2 up to a height of 30 cm";
        assert!(unfaithful(said, "A liquid of refractive index of 1.5 up to a height of 30 cm.").is_some());
        assert!(unfaithful("about thirty percent faster", "About 40% faster.").is_some());
    }

    #[test]
    fn sound_alikes_spelled_differently_count_as_made_up() {
        // The price of catching made-up words: the transcript as heard goes in.
        assert!(unfaithful("then deploy it to for sell", "Then deploy it to Vercel.").is_some());
    }

    #[test]
    fn dropped_speech_is_caught() {
        let said = "And Intel Mac would run the same, so we don't bother about that. Do it fast. I need it done fast.";
        assert!(unfaithful(said, "And Intel Mac would run the same, so we don't bother about that.").is_some());
        let said = "So a few things, we don't have the graph for the usage, and I need to sign in again after a restart. \
                    At least six stat and the rename thing is not working.";
        assert!(unfaithful(said, "At least six stat and the rename thing is not working.").is_some());
    }

    #[test]
    fn real_cleanups_pass() {
        for (said, out) in [
            ("Let's meet at 5. Wait, no, at 6.", "Let's meet at 6."),
            ("Um, I think we should, uh, ship it on Monday.", "I think we should ship it on Monday."),
            ("I'll set up the post gress database and ask Claw about it", "I'll set up the Postgres database and ask Claude about it."),
            ("my tasks today are first buy milk second call the bank third fix the login bug", "My tasks today are:\n1. Buy milk\n2. Call the bank\n3. Fix the login bug"),
            ("it costs twenty dollars no wait 25 dollars and we need three chairs", "It costs 25 dollars and we need 3 chairs."),
            ("okay let me think the main goal for this sprint is shipping the billing page", "The main goal for this sprint is shipping the billing page."),
            ("can you refer to the settings file and check why it crashes", "Can you refer to @src/settings.py and check why it crashes?"),
            ("the API is about thirty percent faster now", "The API is about 30% faster now."),
            ("we need twenty five chairs and two thousand plates by five thirty", "We need 25 chairs and 2000 plates by 5:30."),
            ("the dots are pretty similar to react bits", "The dots are pretty similar to ReactBits."),
            ("please review the contract by Monday <quiet>yeah yeah I'll be there in a minute</quiet> and send me your notes", "Please review the contract by Monday and send me your notes."),
        ] {
            assert_eq!(unfaithful(said, out), None, "{said}");
        }
    }

    #[test]
    fn long_dictations_are_never_an_aside() {
        assert!(believable_aside("hang on, can you turn the music down"));
        assert!(believable_aside("<quiet>one sec I'm on a call, I'll call you right back after this meeting is done okay</quiet>"));
        assert!(!believable_aside("Much better, this is much better. Now we can add an SVG filter blob from bottom to top, 100 to 0%. Do it."));
    }

    #[test]
    fn as_heard_has_no_tags() {
        assert_eq!(as_heard("send it <quiet>hmm</quiet>  now"), "send it hmm now");
    }

    #[test]
    fn haiku_request() {
        let b = haiku_body("rules", "um hi", &["src/a.rs"], 100);
        assert_eq!(b["model"], "claude-haiku-5-5");
        assert_eq!(b["thinking"]["type"], "disabled");
        assert!(b.get("temperature").is_none());
        let messages = b["messages"].as_array().unwrap();
        assert_eq!(messages.last().unwrap()["content"], "<dictation>um hi</dictation>\n<files>\nsrc/a.rs\n</files>");
        // The examples are cached, the dictation after them isn't.
        assert_eq!(messages[messages.len() - 2]["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(messages.len(), 2 * (EXAMPLES.len() + ASIDE_EXAMPLES.len()) + 1);
    }
}
