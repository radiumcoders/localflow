//! `localflow --eval`: run cases.txt through the cleanup and score it.

use std::sync::LazyLock;

use regex::Regex;

use crate::{Localflow, NO_FILES, Result};

/// Raw transcript => expected output, one per line ("\n" for line breaks).
pub const CASES: &str = include_str!("../../../../cases.txt");

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

pub fn run(lf: &Localflow, model: &str, review_model: &str) -> Result<()> {
    let cases = cases();
    lf.clean_with("warm up", model, review_model, NO_FILES)?; // load the models so the first case isn't timed cold
    let (mut drafts_ok, mut finals_ok, mut fixed, mut broke, mut changed) = (0, 0, 0, 0, 0);
    let (mut ms_draft, mut ms_review) = (0, 0);
    println!("model: {model}, review: {review_model}\n");
    let show = |t: &str| t.replace('\n', " | ").chars().take(300).collect::<String>();
    for (raw, expected) in &cases {
        let _ = lf.warm(model); // as when dictating: the hotkey warms the model while you speak
        let r = lf.clean_with(raw, model, review_model, NO_FILES)?;
        let (d_ok, f_ok) = (grade(&r.draft, expected), grade(&r.text, expected));
        drafts_ok += d_ok as usize;
        finals_ok += f_ok as usize;
        changed += (r.text != r.draft) as usize;
        fixed += (f_ok && !d_ok) as usize;
        broke += (d_ok && !f_ok) as usize;
        ms_draft += r.ms_draft;
        ms_review += r.ms_review;
        let mut tag = String::from(if f_ok { "PASS" } else { "FAIL" });
        if d_ok != f_ok {
            tag += if f_ok { " (review fixed)" } else { " (review broke)" };
        }
        let short: String = raw.chars().take(110).collect();
        println!("{tag} {:5}+{:<5}ms [{}]  {short}", r.ms_draft, r.ms_review, r.verdict);
        if !f_ok || d_ok != f_ok {
            println!("           want:  {}", show(expected));
            if d_ok != f_ok {
                println!("           draft: {}", show(&r.draft));
            }
            println!("           final: {}", show(&r.text));
        }
    }
    let n = cases.len().max(1) as u128;
    println!("\ndraft {drafts_ok}/{}, final {finals_ok}/{} (review changed {changed}: fixed {fixed}, broke {broke}); \
              avg {}ms draft + {}ms review", cases.len(), cases.len(), ms_draft / n, ms_review / n);
    Ok(())
}
