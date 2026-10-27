//! `localflow --eval`: run cases.txt through the cleanup and score it.

use std::sync::LazyLock;

use regex::Regex;

use crate::{Localflow, NO_FILES, Result};

/// Raw transcript => expected output, one per line ("\n" for line breaks).
pub const CASES: &str = include_str!("../../../cases.txt");

static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w\s]").unwrap());
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+\.").unwrap());

pub fn cases() -> Vec<(String, String)> {
    CASES.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once("=>"))
        .map(|(raw, expected)| (raw.trim().to_string(), expected.trim().replace("\\n", "\n")))
        .collect()
}

/// Property checks for open-ended cases: "+text" must appear, "-text" must
/// not, "order: a < b < c" must appear in that order, "count: text >= n"
/// must appear at least n times (all case-insensitive).
pub fn check(out: &str, spec: &str) -> bool {
    let low = out.to_lowercase();
    for part in spec.split(';').map(str::trim) {
        if let Some(t) = part.strip_prefix('+') && !low.contains(&t.trim().to_lowercase()) {
            return false;
        }
        if let Some(t) = part.strip_prefix('-') && low.contains(&t.trim().to_lowercase()) {
            return false;
        }
        if let Some(rest) = part.strip_prefix("count:") {
            let (text, n) = rest.rsplit_once(">=").unwrap_or((rest, "0"));
            if low.matches(&text.trim().to_lowercase()).count() < n.trim().parse().unwrap_or(0) {
                return false;
            }
        }
        if let Some(rest) = part.strip_prefix("order:") {
            let pos: Vec<Option<usize>> = rest.split('<').map(|w| low.find(&w.trim().to_lowercase())).collect();
            if pos.contains(&None) || !pos.is_sorted() {
                return false;
            }
        }
    }
    true
}

fn norm(s: &str) -> Vec<String> {
    const DIGITS: [&str; 11] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    NOT_WORD.replace_all(&s.to_lowercase(), "")
        .split_whitespace()
        .map(|w| DIGITS.iter().position(|d| *d == w).map_or(w.to_string(), |n| n.to_string()))
        .collect()
}

fn items(t: &str) -> Vec<Vec<String>> {
    t.lines().filter(|l| LIST_ITEM.is_match(l)).map(norm).collect()
}

pub fn grade(out: &str, expected: &str) -> bool {
    if let Some(spec) = expected.strip_prefix('?') {
        return check(out, spec);
    }
    if !items(expected).is_empty() {
        // List cases: the numbered items must match; the lead-in may be reworded.
        return items(out) == items(expected);
    }
    // Ignore punctuation/case differences; must stay a single line.
    norm(out) == norm(expected) && out.trim().lines().count() == 1
}

pub fn run(lf: &Localflow) -> Result<()> {
    let cases = cases();
    lf.clean("warm up", NO_FILES)?; // load the model so the first case isn't timed cold
    let (mut passed, mut ms) = (0, 0);
    println!("model: {}\n", lf.cfg.model);
    let show = |t: &str| t.replace('\n', " | ").chars().take(300).collect::<String>();
    for (raw, expected) in &cases {
        let _ = lf.warm(); // as when dictating: the hotkey warms the model while you speak
        let r = lf.clean_detailed(raw, NO_FILES)?;
        let ok = grade(&r.text, expected);
        passed += ok as usize;
        ms += r.ms;
        let short: String = raw.chars().take(110).collect();
        println!("{} {:5}ms [{}]  {short}", if ok { "PASS" } else { "FAIL" }, r.ms, r.verdict);
        if !ok {
            println!("           want: {}", show(expected));
            println!("           got:  {}", show(&r.text));
        }
    }
    println!("\n{passed}/{} passed; avg {}ms", cases.len(), ms / cases.len().max(1) as u128);
    Ok(())
}
