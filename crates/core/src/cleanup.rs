//! The cleanup: localflow's own model's answer (or Claude Haiku 5.5's), the
//! checks that fall back to the raw transcript, and Markdown headings small
//! models miss.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
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
/// An email address: its @ is the word "at", not a file reference.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)([a-z0-9._%+-]+)@([a-z0-9-]+(?:\.[a-z0-9-]+)+)"));
static SECTION_LEADIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(?:on|for|about|regarding) (?:the )?([\w'&/ -]{1,30}?)(?: side| front)?:$"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][\w'&/ -]{0,40}$"));
static QUIET: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<quiet>.*?</quiet>"));
static MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\s*(?:\d+\.|[-*]|#+)\s"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[0-9]+(?:\.[0-9]+)?|[a-z0-9]+(?:'[a-z]+)?"));
/// "2,000", and "31st" (already lowercase).
static GROUPED: LazyLock<Regex> = LazyLock::new(|| re(r"[0-9]{1,3}(?:,[0-9]{3})+"));
static ORDINAL: LazyLock<Regex> = LazyLock::new(|| re(r"([0-9])(?:st|nd|rd|th)\b"));

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
/// that drops half a minute of speech as an aside loses it for good. A reply
/// ("yes", "ok") is never an aside, however short.
pub fn believable_aside(text: &str) -> bool {
    WORD.find_iter(&QUIET.replace_all(&text.to_lowercase(), " ")).count() <= 10 && !reply(text)
}

/// Words that are an answer on their own.
const REPLIES: &str = "yes yeah yep yup no nope nah ok okay sure thanks please great cool nice good perfect fine \
                       correct exactly absolutely definitely agreed done alright maybe";
/// And answers of two words that aren't two of those.
const REPLY_PHRASES: &[&str] = &["thank you", "got it", "sounds good", "of course", "not yet", "no problem", "all good", "go ahead"];

/// A word or two that answer someone ("yes", "ok thanks"): little, but meant
/// for the text. The cleanup model takes them for asides.
fn reply(text: &str) -> bool {
    let text = QUIET.replace_all(&text.to_lowercase(), " ").into_owned();
    let said: Vec<&str> = WORD.find_iter(&text).map(|m| m.as_str()).collect();
    matches!(said.len(), 1 | 2)
        && (said.iter().all(|w| REPLIES.split_whitespace().any(|r| r == *w)) || REPLY_PHRASES.contains(&said.join(" ").as_str()))
}

/// A reply as a sentence, for when the model wouldn't write it: "yes" -> "Yes."
fn reply_sentence(text: &str) -> String {
    let said = as_heard(text);
    let mut chars = said.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    let mut sentence = first + chars.as_str();
    if sentence.get(..2).is_some_and(|ok| ok.eq_ignore_ascii_case("ok")) && !sentence[2..].starts_with(char::is_alphanumeric) {
        sentence.replace_range(..2, "OK");
    }
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
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

/// The value of a number word: under a hundred, or a scale. The digits the
/// speech model wrote itself count too.
fn cardinal(word: &str) -> Option<u64> {
    if let Ok(n) = word.parse::<u64>() {
        return (n < 100).then_some(n); // words() has made zero to ten digits
    }
    let small = NUMBERS.iter().position(|n| *n == word).map(|n| n as u64);
    small.or_else(|| BIG_NUMBERS.iter().chain(SCALES).find(|(name, _)| *name == word).map(|&(_, n)| n))
}

/// The value of an ordinal ("thirty first" -> 31st).
fn ordinal(word: &str) -> Option<u64> {
    match word {
        "first" => Some(1),
        "second" => Some(2),
        "third" => Some(3),
        "fifth" => Some(5),
        "eighth" => Some(8),
        "ninth" => Some(9),
        "twelfth" => Some(12),
        // "twentieth", "sixth", "hundredth"
        _ => word.strip_suffix("ieth").and_then(|w| cardinal(&format!("{w}y"))).or_else(|| cardinal(word.strip_suffix("th")?)),
    }
}

fn number(word: &str) -> Option<u64> {
    cardinal(word).or_else(|| ordinal(word))
}

/// What a number said as one ends in so far, for what may come next: a unit
/// after "twenty" or "hundred", not after another unit.
#[derive(Clone, Copy, PartialEq)]
enum Last {
    Nothing,
    Unit,
    /// Ten to nineteen, or digits: nothing under a hundred comes next.
    Whole,
    Tens,
    Hundred,
    Scale,
    /// An ordinal or a zero: the number is over.
    Over,
}

/// Number words said as one number, added up: "one hundred and twenty" is
/// 120, "two thousand five" 2005, "thirty first" 31.
struct Sum {
    total: u64,
    current: u64,
    last: Last,
    /// The smallest scale so far: "two thousand three million" is no number.
    scale: u64,
}

impl Sum {
    fn new() -> Self {
        Sum { total: 0, current: 0, last: Last::Nothing, scale: u64::MAX }
    }

    fn value(&self) -> u64 {
        self.total.saturating_add(self.current)
    }

    /// False, and nothing added, for a word that doesn't go on the number so
    /// far: "one two three" is no 6, "one second" no 3, "twenty thirty" no 50.
    fn push(&mut self, n: u64, ordinal: bool) -> bool {
        use Last::*;
        let fits = match n {
            0 => self.last == Nothing,
            1..=9 => matches!(self.last, Nothing | Tens | Hundred | Scale),
            10..=99 => matches!(self.last, Nothing | Hundred | Scale),
            // "nineteen hundred", "twenty five hundred"
            100 => matches!(self.last, Nothing | Unit | Whole | Tens) && self.current < 100,
            _ => matches!(self.last, Nothing | Unit | Whole | Tens | Hundred) && n < self.scale,
        };
        if !fits {
            return false;
        }
        match n {
            100 => self.current = self.current.max(1).saturating_mul(100),
            n if n > 100 => {
                (self.total, self.current) = (self.total.saturating_add(self.current.max(1).saturating_mul(n)), 0);
                self.scale = n;
            }
            n => self.current = self.current.saturating_add(n),
        }
        self.last = match n {
            _ if ordinal => Over,
            0 => Over,
            1..=9 => Unit,
            10..=99 if n >= 20 && n.is_multiple_of(10) => Tens,
            10..=99 => Whole,
            100 => Hundred,
            _ => Scale,
        };
        true
    }
}

/// Number words read out in a row, a unit joining the tens before it: "four
/// oh four" is 404, "twenty twenty six" 2026, "five thirty" 530, "one two
/// three" 123.
#[derive(Default)]
struct Row {
    digits: String,
    tens: bool,
}

impl Row {
    /// False, and nothing read, for what can't be read out this way: a scale
    /// ("point five million" is .5, then a million), or too long to be a number.
    fn push(&mut self, n: u64) -> bool {
        let joins = self.tens && (1..=9).contains(&n);
        let digits = n.to_string();
        if n >= 100 || self.digits.len() - joins as usize + digits.len() > 18 {
            return false;
        }
        if joins {
            self.digits.pop();
        }
        self.digits += &digits;
        self.tens = n >= 20 && n.is_multiple_of(10);
        true
    }
}

/// A number said in words is this many words at most ("nine hundred ninety
/// nine million ..." is 14): a speech model stuck on "one one one" costs nothing.
const RUN: usize = 20;

/// What the numbers said in words may be written as, each with the words it
/// stands for (from, to): every run of number words and each part of it, as
/// one number added up ("one hundred and twenty" is 120) and read out in a
/// row ("five thirty" is 5, 30 and 530, for "5:30"; never 35), a decimal on
/// the whole number before it ("two point five" is 2.5), and the minutes of
/// "half past nine" (30).
fn spoken_numbers(said: &[String]) -> HashMap<String, Vec<(usize, usize)>> {
    let mut found: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut add = |written: String, from: usize, to: usize| {
        let spans = found.entry(written).or_default();
        if spans.last() != Some(&(from, to)) {
            spans.push((from, to));
        }
    };
    let word = |i: usize| said.get(i).map_or("", String::as_str);
    let zero = |w: &str| matches!(w, "oh" | "o");
    for start in 0..said.len() {
        match (word(start), word(start + 1)) {
            ("half", "past") => add("30".into(), start, start + 2),
            ("quarter", "past") => add("15".into(), start, start + 2),
            ("quarter", "to" | "till") => {
                add("45".into(), start, start + 2);
                if let Some(hour) = number(word(start + 2)).filter(|h| (1..=12).contains(h)) {
                    add(((hour + 10) % 12 + 1).to_string(), start, start + 3); // the hour before
                }
            }
            ("o'clock", _) => add("00".into(), start, start + 1),
            _ => {}
        }
        // Midway through a longer number, where no decimal starts: "ninety
        // nine point nine" is 99.9, never 9.9 or 0.9.
        let midway = start > 0 && {
            let joins = start > 1 && (zero(word(start - 1)) || (word(start - 1) == "and" && number(word(start - 2)).is_some_and(|n| n >= 100)));
            number(word(start - 1)).is_some() || (joins && number(word(start - 2)).is_some())
        };
        // The words so far, as one number added up and as numbers read out in a row.
        let (mut sum, mut row) = (Some(Sum::new()), Some(Row::default()));
        let wholes = |sum: &Option<Sum>, row: &Option<Row>| {
            let read_out = row.iter().map(|row| row.digits.clone()).filter(|digits| !digits.is_empty());
            sum.iter().map(|sum| sum.value().to_string()).chain(read_out).collect::<Vec<String>>()
        };
        let mut i = start;
        while i < said.len().min(start + RUN) {
            // "four oh four": a zero between numbers, not the "oh" of "five, oh, and three more".
            let oh = || i > start && zero(word(i)) && (i + 1..said.len()).find(|j| !zero(word(*j))).is_some_and(|j| number(word(j)).is_some());
            let (n, nth) = match (cardinal(word(i)), ordinal(word(i))) {
                (Some(n), _) => (n, false),
                (None, Some(n)) => (n, true),
                (None, None) if oh() => (0, false),
                // "one hundred and twenty"
                (None, None)
                    if word(i) == "and"
                        && sum.as_ref().is_some_and(|sum| matches!(sum.last, Last::Hundred | Last::Scale))
                        && number(word(i + 1)).is_some_and(|n| n < 100) =>
                {
                    i += 1;
                    continue;
                }
                // "one and a half"
                (None, None) if i > start && (word(i), word(i + 1), word(i + 2)) == ("and", "a", "half") => {
                    for whole in wholes(&sum, &row).into_iter().filter(|_| !midway) {
                        add(format!("{whole}.5"), start, i + 3);
                    }
                    break;
                }
                // "two point five", "point five": the whole of what follows, not "3.1" of "three point one four"
                (None, None) if matches!(word(i), "point" | "dot") => {
                    let (mut decimals, mut end) = (Row::default(), i + 1);
                    while end < said.len().min(start + RUN) && cardinal(word(end)).or(zero(word(end)).then_some(0)).is_some_and(|n| decimals.push(n)) {
                        end += 1;
                    }
                    for whole in wholes(&sum, &row).into_iter().filter(|_| end > i + 1 && !midway) {
                        add(format!("{whole}.{}", decimals.digits), start, end);
                    }
                    break;
                }
                (None, None) => break,
            };
            sum = sum.and_then(|mut sum| sum.push(n, nth).then_some(sum));
            // An ordinal is read out only as the unit of its tens: "twenty first", not "one second".
            row = row.and_then(|mut row| ((!nth || (row.tens && (1..=9).contains(&n))) && row.push(n)).then_some(row));
            if sum.is_none() && row.is_none() {
                break;
            }
            for whole in wholes(&sum, &row) {
                add(whole, start, i + 1);
            }
            row = row.filter(|_| !nth); // and ends it
            i += 1;
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
/// Symbols said as words, which the cleanup writes as the symbol
/// ("example dot com slash docs" -> example.com/docs).
const SYMBOLS: &[&str] = &["dot", "slash", "colon", "dash", "hyphen", "underscore"];

/// Words that take back what was said before them.
const TAKE_BACK: &[&str] = &["wait", "sorry", "actually", "mean", "scratch", "rather", "instead", "oh"];

/// A word near a dropped stretch that says it was replaced or set aside.
fn cue(word: &str) -> bool {
    TAKE_BACK.contains(&word) || matches!(word, "no" | "not" | "hmm" | "think")
}

/// A word that turns what's said around.
fn negation(word: &str) -> bool {
    word.ends_with("n't")
        || matches!(word, "not" | "no" | "never" | "cannot" | "without" | "nor" | "neither" | "none" | "nothing" | "nobody" | "nowhere" | "nope")
}

fn small(word: &str) -> bool {
    FILLER.split(' ').chain(GRAMMAR.split(' ')).any(|w| w == word)
}

/// How many words either side of a change count as near it.
const NEAR: usize = 3;

/// Lowercase words, with what the cleanup legitimately adds or removes (list
/// markers, @file references, muttered parts) left out, number words as
/// digits, and the @ of an email address as the "at" that was said.
fn words(text: &str) -> Vec<String> {
    let text = QUIET.replace_all(text, " ");
    let text = MARKER.replace_all(&text, " ");
    let text = EMAIL.replace_all(&text, "$1 at $2");
    let text = FILE_REF.replace_all(&text, " ").to_lowercase().replace('%', " percent").replace('\u{2019}', "'");
    let text = GROUPED.replace_all(&text, |grouped: &regex::Captures| grouped[0].replace(',', ""));
    let text = ORDINAL.replace_all(&text, "$1");
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

/// A name or term in the speaker's vocabulary, as words, and the ways it's
/// misheard (words.txt: 'Kubernetes: often heard as "cube and eighties"').
struct Term {
    words: Vec<String>,
    heard: Vec<Vec<String>>,
}

fn terms(vocabulary: &[String]) -> Vec<Term> {
    let terms = vocabulary.iter().map(|line| crate::vocab::term(line));
    terms.map(|(term, heard)| Term { words: words(&term), heard: heard.iter().map(|h| words(h)).filter(|h| !h.is_empty()).collect() }).collect()
}

impl Term {
    /// Whether `word` is this term's and the term was said in `near`: as one
    /// of the ways it's misheard, or itself, spelled or spaced another way
    /// ("next JS" -> Next.js). The vocabulary makes a small model write its
    /// terms where nothing like them was said, so being in it isn't enough.
    fn explains(&self, word: &str, near: &[String]) -> bool {
        let said_as = |heard: &Vec<String>| near.windows(heard.len()).any(|said| said == heard.as_slice());
        let respelled = || (1..=4).any(|k| near.windows(k).any(|said| alike(&self.words.concat(), &said.concat()) >= 0.75));
        self.words.iter().any(|w| w == word) && (self.heard.iter().any(said_as) || respelled())
    }

    /// Where in `said` this term was misheard, when the cleanup wrote the term: (from, to).
    fn misheard<'a>(&'a self, said: &'a [String], kept: &'a HashSet<&String>) -> impl Iterator<Item = (usize, usize)> + 'a {
        let written = self.words.iter().all(|w| kept.contains(w));
        let places = move |heard: &'a Vec<String>| {
            (0..said.len()).filter(move |i| said[*i..].starts_with(heard)).map(move |i| (i, i + heard.len()))
        };
        self.heard.iter().filter(move |_| written).flat_map(places)
    }
}

/// Casual speech, and what the cleanup may write for it.
const CASUAL: &[(&str, &str)] = &[
    ("gonna", "going to"), ("wanna", "want to"), ("gotta", "got to"), ("kinda", "kind of"), ("sorta", "sort of"),
    ("lemme", "let me"), ("gimme", "give me"), ("cuz", "because"), ("cause", "because"), ("yeah", "yes"), ("yep", "yes"),
    ("ya", "you"), ("til", "until"),
];

/// Words that sound the same: writing the right one is grammar.
const SOUND_ALIKE: &[&str] = &[
    "there their they're", "your you're", "to too", "then than", "whose who's", "were we're where", "affect effect",
    "accept except", "lose loose", "hear here", "weather whether",
];

/// What a word may be a form of: itself and what its ending is on, less a
/// final e or y or doubled letter ("write", "writing" and "writes" are all
/// "writ"; "shipping" is "ship", "studies" is "stud").
fn stems(word: &str) -> Vec<String> {
    let bare = |stem: &str| {
        let mut stem: Vec<char> = stem.chars().collect();
        if stem.len() > 3 && matches!(stem.last(), Some('e' | 'y')) {
            stem.pop();
        }
        if stem.len() > 3 && stem[stem.len() - 1] == stem[stem.len() - 2] {
            stem.pop();
        }
        stem.into_iter().collect::<String>()
    };
    let endings = ["'s", "s", "es", "ed", "d", "ing", "ies", "ied"];
    let cut = endings.iter().filter_map(|ending| word.strip_suffix(ending)).filter(|stem| stem.chars().count() >= 3);
    std::iter::once(word).chain(cut).map(bare).collect()
}

/// The same word in another form: "ship" and "shipping", not "dictation" and "diction".
fn same_word(a: &str, b: &str) -> bool {
    let b = stems(b);
    stems(a).iter().any(|stem| b.contains(stem))
}

/// The same word spelled another way: a letter apart ("colour" / "color",
/// "there" / "their", "its" / "it's"). A short word may only gain or lose
/// one ("Jon" / "John"): "test" isn't "text", nor "now" "not".
fn respelled(a: &str, b: &str) -> bool {
    let letters = |w: &str| w.chars().filter(|c| *c != '\'').collect::<Vec<char>>();
    let (a, b) = (letters(a), letters(b));
    let same: usize = SequenceMatcher::new(&a, &b).matching_blocks().iter().map(|m| m.size).sum();
    let (short, long) = (a.len().min(b.len()), a.len().max(b.len()));
    match short {
        0..=2 => a == b,
        3..=4 => same == short && long - short <= 1,
        _ => long - same <= 1,
    }
}

/// A word in the cleanup that the speaker said, near enough. Anywhere in
/// what they said: the word itself (a list may change its order) or another
/// form of it. In `near`, the words around where it was written: the word
/// respelled ("colour" -> color), words said apart ("post gress" ->
/// Postgres), casual speech written out ("gonna" -> going to), one that
/// sounds the same ("their" -> they're), or a term of their vocabulary. A
/// small word for grammar always. A number has to be one they said (in
/// words, `numbers`, or digits). Looking a bit like some word they said
/// somewhere isn't enough ("then" doesn't explain "Python").
fn explained(word: &str, said: &[String], near: &[String], numbers: &HashMap<String, Vec<(usize, usize)>>, terms: &[Term]) -> bool {
    if small(word) || said.iter().any(|w| w == word) || terms.iter().any(|t| t.explains(word, near)) {
        return true;
    }
    if word.starts_with(|c: char| c.is_ascii_digit()) {
        return numbers.contains_key(word);
    }
    let same_sound = |group: &&str| group.split(' ').any(|w| w == word) && near.iter().any(|n| group.split(' ').any(|w| w == n));
    near.iter().any(|w| respelled(word, w))
        || (2..=3).any(|k| near.windows(k).any(|apart| alike(word, &apart.concat()) >= 0.75))
        || CASUAL.iter().any(|(casual, full)| full.split(' ').any(|w| w == word) && near.iter().any(|w| w == casual))
        || SOUND_ALIKE.iter().any(same_sound)
        || said.iter().any(|w| same_word(word, w))
}

/// What the cleanup got wrong, if anything: words the speaker never said
/// ("slash got shaders skill" -> "Claude Code", "root 2" -> "1.5"), or a
/// stretch of what they said left out, with no correction near it ("Do it
/// fast. I need it done fast." gone). Small models do both; the transcript as
/// heard is better than either. A negation left out, or put in, turns the
/// sentence around however little else changed ("the test is not passing"
/// -> "The test is passing."). `vocabulary`: the speaker's names and terms
/// (words.txt lines), which explain a word that sounds nothing like what was
/// heard.
pub fn unfaithful(text: &str, out: &str, vocabulary: &[String]) -> Option<String> {
    let (said, wrote) = (words(text), words(out));
    let kept: HashSet<&String> = wrote.iter().collect();
    let numbers = spoken_numbers(&said);
    let terms = terms(vocabulary);
    let misheard: Vec<(usize, usize)> = terms.iter().flat_map(|t| t.misheard(&said, &kept)).collect();
    let written = wrote.as_slice();
    for op in SequenceMatcher::new(&said, &wrote).opcodes() {
        let new = &wrote[op.j1..op.j2];
        let near = &said[op.i1.saturating_sub(NEAR)..(op.i2 + NEAR).min(said.len())];
        if matches!(op.tag, Tag::Replace | Tag::Insert) {
            // As many negations as the speaker said here ("do not" -> "don't"): one more is made up,
            // however near another is ("I can't believe it works" -> "... it doesn't work").
            let said_here = said[op.i1..op.i2].iter().filter(|w| negation(w)).count();
            let negations: Vec<usize> = (op.j1..op.j2).filter(|j| negation(&wrote[*j])).collect();
            // One word said, written as two or three ("nextjs" -> Next.js, "cannot" -> can not).
            let split = |j: usize| {
                let around = |k: usize| (j.saturating_sub(k - 1)..=j).filter_map(move |from| written.get(from..from + k));
                (2..=3).any(|k| around(k).any(|parts| near.contains(&parts.concat())))
            };
            let made_up = |j: &usize| match negations.iter().position(|n| n == j) {
                Some(nth) => nth >= said_here,
                None => !explained(&wrote[*j], &said, near, &numbers, &terms) && !split(*j),
            };
            let added: Vec<&str> = (op.j1..op.j2).filter(made_up).map(|j| wrote[j].as_str()).collect();
            if !added.is_empty() {
                return Some(format!("added {:?}", added.join(" ")));
            }
        }
        if matches!(op.tag, Tag::Replace | Tag::Delete) {
            if let Some(not) = flipped(&said, op.i1, op.i2, new) {
                return Some(format!("dropped {not:?}"));
            }
            // Number words the cleanup wrote as digits aren't lost, nor a misheard term it wrote right.
            let digits = new.iter().filter_map(|w| numbers.get(w)).flatten();
            let accounted: Vec<&(usize, usize)> = digits.chain(&misheard).collect();
            let lost: Vec<&str> = (op.i1..op.i2)
                .filter(|i| !small(&said[*i]) && !SYMBOLS.contains(&said[*i].as_str()) && !kept.contains(&said[*i]))
                .filter(|i| !accounted.iter().any(|(from, to)| (*from..*to).contains(i)))
                .map(|i| said[i].as_str())
                .collect();
            if lost.len() >= 3 && !near.iter().any(|w| cue(w)) {
                return Some(format!("dropped {:?}", lost.join(" ")));
            }
        }
    }
    None
}

/// The negation the cleanup left out of said[i1..i2] (which it wrote as
/// `new`), if that turns the sentence around. It doesn't when what was
/// negated went with it ("it's not good, it's great" -> "It's great."), when
/// it was said twice, or when "no" takes back what came before it ("at
/// three, no, four").
fn flipped<'a>(said: &'a [String], i1: usize, i2: usize, new: &[String]) -> Option<&'a str> {
    let gone = &said[i1..i2];
    let negations = |words: &[String]| words.iter().filter(|w| negation(w)).count();
    let (before, after) = (&said[..i1], &said[i2..]);
    let repeated = after.starts_with(gone)
        || before.ends_with(gone)
        || gone.iter().all(|w| after.first() == Some(w) || before.last() == Some(w));
    if negations(gone) <= negations(new) || repeated {
        return None;
    }
    let near = &said[i1.saturating_sub(NEAR)..(i2 + NEAR).min(said.len())];
    let taken_back = near.iter().any(|w| TAKE_BACK.contains(&w.as_str()));
    let content = |w: &String| !small(w) && !negation(w);
    (0..gone.len()).filter(|&k| negation(&gone[k])).map(|k| (k, gone[k].as_str())).find_map(|(k, not)| {
        let went_with_it = gone[k + 1..].iter().any(content);
        let takes_back = not == "no" && (taken_back || gone[..k].iter().any(content));
        (!went_with_it && !takes_back).then_some(not)
    })
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
            if out == ASIDE && reply(text) {
                result.text = reply_sentence(text); // "yes" is no aside: "Yes."
            }
            "implausible".into() // keep what the speaker said
        } else if let Some(why) = (!by_haiku).then(|| unfaithful(text, &out, &self.vocabulary())).flatten() {
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

    /// With no vocabulary of the speaker's.
    fn unfaithful(text: &str, out: &str) -> Option<String> {
        super::unfaithful(text, out, &[])
    }

    #[test]
    fn the_vocabulary_explains_what_it_says_is_misheard() {
        let vocabulary: Vec<String> = [
            "Kubernetes: often heard as \"cube and eighties\"",
            "Claude Code",
            "Next.js",
            "Pine Script: TradingView's scripting language, often heard as \"Py script\" or \"pie script\"",
            "PostgreSQL: the database, often heard as \"post gress queue well\"",
        ]
        .map(String::from)
        .to_vec();
        let (said, out) = ("deploy it to cube and eighties tonight", "Deploy it to Kubernetes tonight.");
        assert_eq!(unfaithful(said, out), Some("added \"kubernetes\"".into()));
        for (said, out) in [
            (said, out),
            ("code that into a Py script so I can backtest it", "Code that into a Pine Script so I can backtest it."),
            ("check the next JS docs", "Check the Next.js docs."),
            // The words it was misheard as aren't dropped ones.
            ("set up post gress queue well on the server tonight", "Set up PostgreSQL on the server tonight."),
        ] {
            assert_eq!(super::unfaithful(said, out, &vocabulary), None, "{said}");
        }
        // A term nothing like what was said is still made up, and so is one whose mishearing wasn't said here.
        let said = "create some cool random shaders with slash got shaders skill and use some random seeds";
        assert!(super::unfaithful(said, "Create some cool random shaders with Claude Code and use some random seeds.", &vocabulary).is_some());
        assert!(super::unfaithful("deploy it to the staging box tonight", "Deploy it to Kubernetes tonight.", &vocabulary).is_some());
        let said = "a cube and eighties poster is on the wall and then we deploy it to the staging box at the end of the day";
        let out = "A cube and eighties poster is on the wall and then we deploy it to Kubernetes at the end of the day.";
        assert!(super::unfaithful(said, out, &vocabulary).is_some());
    }

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
    fn another_word_that_looks_alike_is_made_up() {
        let python = ["Python".to_string()];
        for (said, out) in [
            ("Dictation", "Diction."),
            // "Python" was explained by "then", a word nowhere near it.
            ("and then I will code that thing into a Py script so that I can backtest it", "And then I will code that thing into a Python script so that I can backtest it."),
            ("run the test again", "Run the text again."),
            ("the computer is slow", "The company is slow."),
            ("we had a long conversation", "We had a long conservation."),
            ("the form is broken", "The from is broken."),
            ("I'll walk you through it", "I'll talk you through it."),
        ] {
            assert!(unfaithful(said, out).is_some(), "{said}");
            assert!(super::unfaithful(said, out, &python).is_some(), "{said}");
        }
    }

    #[test]
    fn the_same_word_respelled_or_in_another_form_passes() {
        for (said, out) in [
            ("i think we should ship it on monday", "I think we should ship it on Monday."),
            ("I'm gonna ship it tomorrow", "I'm going to ship it tomorrow."),
            ("I wanna try it cuz it's kinda faster", "I want to try it because it's kind of faster."),
            ("the colour is off", "The color is off."),
            ("I think there car is red", "I think their car is red."),
            ("its working now and were done", "It's working now and we're done."),
            ("I think their going to announce it", "I think they're going to announce it."),
            ("it's better then the old one", "It's better than the old one."),
            ("send it to Jon", "Send it to John."),
            ("so first replying to the emails and second I need to be fixing the build", "1. Reply to the emails\n2. Fix the build"),
            ("we use nextjs and the backend cannot keep up", "We use Next.js and the back end can not keep up."),
            ("first I will buy groceries and then shoes oh wait first shoes", "1. Shoes\n2. Groceries"),
        ] {
            assert_eq!(unfaithful(said, out), None, "{said}");
        }
        for (a, b) in [("ship", "shipping"), ("ships", "shipped"), ("write", "writing"), ("study", "studies"), ("need", "needs"), ("pass", "passes"), ("reply", "replied"), ("client", "client's")] {
            assert!(same_word(a, b) && same_word(b, a), "{a} {b}");
        }
        for (a, b) in [("dictation", "diction"), ("then", "they"), ("test", "text"), ("computer", "company"), ("conversation", "conservation")] {
            assert!(!same_word(a, b) && !respelled(a, b), "{a} {b}");
        }
    }

    #[test]
    fn repeated_number_words_dont_overflow() {
        let said = format!("{} billion dollars", "hundred ".repeat(30));
        assert!(unfaithful(&said, &format!("{said}.")).is_none());
        // Nor take long, nor panic, when a speech model is stuck on one.
        for stuck in ["nine ", "ninety nine thousand ", "one hundred and ", "two point ", "half past quarter to ", "twentieth oh "] {
            let said = stuck.repeat(300);
            assert!(unfaithful(&said, &said).is_none());
            assert!(unfaithful(&said, "9999999999999999999999999999999999999999 and 18446744073709551616.").is_some());
        }
    }

    #[test]
    fn numbers_written_as_digits_pass() {
        for (said, out) in [
            ("one hundred twenty", "120."),
            ("it costs one hundred and twenty dollars", "It costs $120."),
            ("three hundred sixty five days", "365 days."),
            ("we have two thousand and five users", "We have 2,005 users."),
            ("the ratio is two point five", "The ratio is 2.5."),
            ("version three point one four", "Version 3.14."),
            ("ninety nine point nine percent uptime", "99.9% uptime."),
            ("it weighs point five kilos", "It weighs 0.5 kilos."),
            ("it takes one and a half hours", "It takes 1.5 hours."),
            ("the deadline is in twenty twenty six", "The deadline is in 2026."),
            ("we launch in twenty twenty seven", "We launch in 2027."),
            ("back in nineteen ninety nine", "Back in 1999."),
            ("it's due on the thirty first", "It's due on the 31st."),
            ("on the twenty fifth of March", "On the 25th of March."),
            ("the page returns a four oh four", "The page returns a 404."),
            ("room two oh four", "Room 204."),
            ("call me at five five five one two three four", "Call me at 555-1234."),
            ("my number is five five five one two three four", "My number is 555-1-234."),
            ("let's meet at half past nine", "Let's meet at 9:30."),
            ("see you at quarter to nine", "See you at 8:45."),
            ("see you at nine o'clock", "See you at 9:00."),
            ("count one two three", "Count 1, 2, 3."),
            ("the code is one two three", "The code is 123."),
            ("give me one second", "Give me 1 second."),
            ("a thirty second delay", "A 30-second delay."),
            ("in nineteen hundred", "In 1900."),
            ("twenty five hundred dollars", "$2,500."),
            ("two thousand five hundred and one", "2,501."),
            ("four oh four point five", "404.5."),
            ("the third of March", "The 3rd of March."),
            // The scale after a decimal is no part of it.
            ("it grew by one point five million", "It grew by 1.5 million."),
            ("about two point five thousand users", "About 2.5 thousand users."),
        ] {
            assert_eq!(unfaithful(said, out), None, "{said}");
        }
    }

    #[test]
    fn wrong_digits_are_caught() {
        // The first three are localflow-clean's own.
        for (said, out) in [
            ("two thousand and five", "2,050."),
            ("call me at five five five one two three four", "Call me at 5:55."),
            ("version three point one four", "Version 3.1.4."),
            ("one hundred twenty", "130."),
            ("the deadline is in twenty twenty six", "The deadline is in 2025."),
            ("let's meet at half past nine", "Let's meet at 9:15."),
            ("open ports eighty and four four three and eight thousand", "Open ports 80."),
            // Numbers in a row aren't added up, a duration is no ordinal, a
            // decimal is on the whole number, and "oh" isn't always a zero.
            ("count one two three", "Count 6."),
            ("give me one second", "Give me 3."),
            ("give me one second", "Give me 12."),
            ("ninety nine point nine percent", "9.9%."),
            ("ninety nine point nine percent", "0.9%."),
            ("one hundred and five point two", "5.2."),
            ("twenty one and a half", "1.5."),
            ("I need five oh and three more", "I need 50 and 3 more."),
            ("twenty thirty", "50."),
            ("two dozen eggs", "14 eggs."),
        ] {
            assert!(unfaithful(said, out).is_some(), "{said}");
        }
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
    fn a_negation_left_out_or_put_in_is_caught() {
        for (said, out) in [
            ("the test is not passing", "The test is passing."),
            ("do not merge this", "Merge this."),
            ("actually the test is not passing", "The test is passing."),
            ("I can't make it on Friday", "I can make it on Friday."),
            ("there are no errors in the build", "There are errors in the build."),
            ("we never deploy on Fridays", "We deploy on Fridays."),
            ("it works without the flag", "It works with the flag."),
            ("the test is not um passing", "The test is passing."),
            ("the test is passing", "The test is not passing."),
            ("merge this", "Don't merge this."),
            ("it\u{2019}s ready so don\u{2019}t wait", "It's ready so wait."),
            // Another negation nearby doesn't make this one said.
            ("I can't believe it works", "I can't believe it doesn't work."),
            ("it's not ready and it works", "It's not ready and it never works."),
            ("I don't think it works", "I think it doesn't work."),
        ] {
            assert!(unfaithful(said, out).is_some(), "{said}");
        }
        assert_eq!(unfaithful("do not merge this", "Merge this."), Some("dropped \"not\"".into()));
        assert_eq!(unfaithful("I can't believe it works", "I can't believe it doesn't work."), Some("added \"doesn't\"".into()));
    }

    #[test]
    fn a_negation_corrected_or_reworded_passes() {
        for (said, out) in [
            ("let's meet at three, no, four", "Let's meet at four."),
            ("at three no four", "At four."),
            ("send the invoice on monday no tuesday", "Send the invoice on Tuesday."),
            ("It costs 20 dollars. No wait, 25 dollars.", "It costs 25 dollars."),
            ("Buy apples, bananas, and oranges. Scratch that, no bananas.", "Buy apples and oranges."),
            ("the demo is not good it is great", "The demo is great."),
            ("it's not on monday it's on tuesday", "It's on Tuesday."),
            ("the food here is decent actually this is not decent this is amazing", "The food here is amazing."),
            ("do not merge this", "Don't merge this."),
            ("I cannot make it", "I can't make it."),
            ("it is not not ready", "It is not ready."),
            ("I don't I don't know", "I don't know."),
            ("no no no that's wrong", "No, that's wrong."),
            ("the retry logic never backs off", "The retry logic never backs off."),
        ] {
            assert_eq!(unfaithful(said, out), None, "{said}");
        }
    }

    #[test]
    fn spoken_addresses_are_the_written_ones() {
        for (said, out) in [
            ("send it to jay at example dot com", "Send it to jay@example.com."),
            ("email me at jay dot smith at example dot com", "Email me at jay.smith@example.com."),
            ("send it to jay@example.com today", "Send it to jay@example.com today."),
            ("go to example dot com slash docs", "Go to example.com/docs."),
            ("it's at https colon slash slash example dot com", "It's at https://example.com."),
        ] {
            assert_eq!(unfaithful(said, out), None, "{said}");
        }
        assert!(unfaithful("send it to jay at example dot com", "Send it to jay@gmail.com.").is_some());
        assert!(unfaithful("send it to jay", "Send it to jay@example.com.").is_some());
        // A file reference is still no word of the speaker's.
        assert_eq!(words("check @src/settings.py and jay@example.com"), ["check", "and", "jay", "at", "example", "com"]);
    }

    #[test]
    fn real_cleanups_pass() {
        for (said, out) in [
            ("Let's meet at 5. Wait, no, at 6.", "Let's meet at 6."),
            ("Um, I think we should, uh, ship it on Monday.", "I think we should ship it on Monday."),
            ("I'll set up the post gress database and ask Claude about it", "I'll set up the Postgres database and ask Claude about it."),
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
    fn a_reply_is_never_an_aside() {
        for said in ["yes", "Yes.", "ok", "Okay.", "yeah", "no", "sure", "thanks", "ok thanks", "Thank you."] {
            assert!(reply(said) && !believable_aside(said), "{said}");
        }
        for said in ["hmm let me think", "hmm", "um okay", "one sec", "<quiet>yes</quiet>", "yes I think so", "wait"] {
            assert!(!reply(said) && believable_aside(said), "{said}");
        }
        for (said, written) in [("yes", "Yes."), ("ok", "OK."), ("okay", "Okay."), ("Yes.", "Yes."), ("ok thanks", "OK thanks."), ("sure!", "Sure!")] {
            assert_eq!(reply_sentence(said), written);
        }
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
