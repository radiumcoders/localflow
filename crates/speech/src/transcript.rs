//! What the speech engine heard: the text, in segments with their timing and
//! loudness. Segments said much more quietly than the rest (muttering to
//! yourself, someone talking in the background) are marked `quiet`, so the
//! cleanup can leave them out.

use crate::audio::RATE;

/// One stretch of speech.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Seconds from the start of the recording.
    pub start_s: f32,
    pub end_s: f32,
    pub text: String,
    /// How loud it was said, 0..1 (the level meter's scale).
    pub level: f32,
    /// How loud it was said in dBFS (RMS of its louder moments): -20 is
    /// close-up speech, -60 near silence. Unlike `level`, never capped.
    pub db: f32,
    /// Much quieter than the dictation's main voice.
    pub quiet: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub segments: Vec<Segment>,
}

/// How much quieter (dB) than the main voice a segment has to be to count
/// as quiet. A dictation's own phrases stay within ~3 dB of each other
/// (measured on real ones), leaning back or turning your head costs ~3-6 dB;
/// muttering under your breath or a voice across the room, 12-25 dB.
pub const QUIET_DROP_DB: f32 = 9.0;
/// Less speech than this and there's no "main voice" to compare with.
const MIN_SPEECH_S: f32 = 1.5;

impl Transcript {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn has_quiet(&self) -> bool {
        self.segments.iter().any(|s| s.quiet)
    }

    /// The text for the cleanup: with `mark_quiet`, quiet segments are
    /// wrapped in `<quiet>…</quiet>` (the cleanup drops them unless they
    /// clearly belong to the text).
    pub fn for_cleanup(&self, mark_quiet: bool) -> String {
        if !mark_quiet || !self.has_quiet() {
            return self.text.clone();
        }
        join(self.segments.iter().map(|s| if s.quiet { format!("<quiet>{}</quiet>", s.text) } else { s.text.clone() }))
    }

    /// From segments in time order: drops empty ones, sets the text.
    pub fn from_segments(segments: Vec<Segment>) -> Transcript {
        let segments: Vec<Segment> = segments.into_iter().filter(|s| !s.text.is_empty()).collect();
        Transcript { text: join(segments.iter().map(|s| s.text.clone())), segments }
    }
}

fn join(parts: impl Iterator<Item = String>) -> String {
    parts.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A word with its timing, as the engine returns it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Word {
    pub text: String,
    pub start_s: f32,
    pub end_s: f32,
    /// Which stretch of speech (between pauses the voice detector found) it's in.
    pub span: usize,
}

/// Silence, in dBFS.
const FLOOR_DB: f32 = -100.0;

/// Loudness of the speech in samples[start..end] (seconds), in dBFS: the RMS
/// of its louder 20 ms frames (the 75th percentile), so pauses between words
/// don't pull it down.
pub(crate) fn speech_db(samples: &[i16], start_s: f32, end_s: f32) -> f32 {
    let mut frames = frames_db(samples, &[(start_s, end_s)]);
    if frames.is_empty() {
        return FLOOR_DB;
    }
    frames.sort_by(f32::total_cmp);
    frames[(frames.len() * 3 / 4).min(frames.len() - 1)]
}

/// The loudness (dBFS) of each 20 ms frame in the `ranges` (seconds).
fn frames_db(samples: &[i16], ranges: &[(f32, f32)]) -> Vec<f32> {
    let at = |s: f32| ((s.max(0.0) * RATE as f32) as usize).min(samples.len());
    let mut frames = Vec::new();
    for &(a, b) in ranges {
        let (a, b) = (at(a), at(b));
        if b > a {
            frames.extend(samples[a..b].chunks(RATE as usize / 50).map(|f| {
                let sum: f64 = f.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
                let rms = (sum / f.len() as f64).sqrt() / 32768.0;
                if rms > 0.0 { (20.0 * rms.log10()).max(f64::from(FLOOR_DB)) as f32 } else { FLOOR_DB }
            }));
        }
    }
    frames
}

/// Frames this far below the recording's loudest speech are pauses and
/// background, not voice (a quiet aside is 10-25 dB down).
const GATE_DB: f32 = 30.0;

/// Loudness (dBFS) of the voice in start..end: only where the voice detector
/// heard speech (`speech`), and only frames above `gate` (word timings run
/// into the pauses around them, which would make a phrase sound quieter than
/// it was). None with under 0.1 s of voice in it.
fn voice_db(samples: &[i16], start_s: f32, end_s: f32, speech: &[(f32, f32)], gate: f32) -> Option<f32> {
    let ranges: Vec<(f32, f32)> = speech.iter().map(|&(a, b)| (a.max(start_s), b.min(end_s))).collect();
    let mut frames: Vec<f32> = frames_db(samples, &ranges).into_iter().filter(|&f| f >= gate).collect();
    if frames.len() < 5 {
        return None;
    }
    frames.sort_by(f32::total_cmp);
    Some(frames[(frames.len() * 3 / 4).min(frames.len() - 1)])
}

/// dBFS on the level meter's 0..1 scale (`audio::level`: 25 to 70 dB over an
/// RMS of 1).
pub(crate) fn db_to_level(db: f32) -> f32 {
    ((db + 20.0 * 32768f32.log10() - 25.0) / 45.0).clamp(0.0, 1.0)
}

/// Words to phrases: a new phrase after a pause (one the voice detector
/// found, or `gap` seconds between words), or where a sentence ends. Each
/// phrase gets its loudness, measured where there's speech (`speech`, from
/// the voice detector). A phrase too short to measure gets its neighbor's.
pub(crate) fn phrases(words: &[Word], samples: &[i16], gap: f32, speech: &[(f32, f32)]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut ended = true;
    let mut span = 0;
    for w in words {
        let text = w.text.trim();
        if text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if !ended && w.span == span && w.start_s - last.end_s < gap => {
                last.text.push(' ');
                last.text.push_str(text);
                last.end_s = w.end_s.max(last.end_s);
            }
            _ => out.push(Segment {
                start_s: w.start_s,
                end_s: w.end_s,
                text: text.to_string(),
                level: 0.0,
                db: FLOOR_DB,
                quiet: false,
            }),
        }
        ended = text.ends_with(['.', '?', '!']);
        span = w.span;
    }
    let mut all = frames_db(samples, speech);
    all.sort_by(f32::total_cmp);
    let loudest = all.get(all.len().saturating_sub(1).min(all.len() * 95 / 100)).copied().unwrap_or(FLOOR_DB);
    let gate = loudest - GATE_DB;
    let measured: Vec<Option<f32>> = out.iter().map(|p| voice_db(samples, p.start_s, p.end_s, speech, gate)).collect();
    for (i, p) in out.iter_mut().enumerate() {
        let nearest = (1..measured.len())
            .flat_map(|d| [i.checked_sub(d), Some(i + d)])
            .flatten()
            .find_map(|j| measured.get(j).copied().flatten());
        p.db = measured[i].or(nearest).unwrap_or(FLOOR_DB);
        p.level = db_to_level(p.db);
    }
    out
}

/// Mark the segments said much more quietly than the main voice. The main
/// voice is the level that half of the speech (by time) reaches, so the
/// quiet part can never be most of what was said. Nothing is marked with
/// little speech, a single segment, or when every segment is quiet.
pub(crate) fn mark_quiet(segments: &mut [Segment]) {
    for s in segments.iter_mut() {
        s.quiet = false;
    }
    let total: f32 = segments.iter().map(|s| s.end_s - s.start_s).sum();
    if segments.len() < 2 || total < MIN_SPEECH_S {
        return;
    }
    let mut by_level: Vec<&Segment> = segments.iter().collect();
    by_level.sort_by(|a, b| b.db.total_cmp(&a.db));
    let mut acc = 0.0;
    let mut main = by_level[0].db;
    for s in by_level {
        acc += s.end_s - s.start_s;
        main = s.db;
        if acc >= total / 2.0 {
            break;
        }
    }
    for s in segments.iter_mut() {
        s.quiet = s.db <= main - QUIET_DROP_DB;
    }
    if segments.iter().all(|s| s.quiet) {
        segments.iter_mut().for_each(|s| s.quiet = false);
    }
}

/// Adjacent segments both quiet (or both not) become one.
pub(crate) fn merge_runs(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for s in segments {
        match out.last_mut() {
            Some(last) if last.quiet == s.quiet => {
                let (a, b) = (last.end_s - last.start_s, s.end_s - s.start_s);
                if a + b > 0.0 {
                    last.db = (last.db * a + s.db * b) / (a + b);
                    last.level = db_to_level(last.db);
                }
                last.text = format!("{} {}", last.text, s.text);
                last.end_s = s.end_s;
            }
            _ => out.push(s),
        }
    }
    out
}

/// Sounds whisper writes down that weren't words: "[BLANK_AUDIO]",
/// "(music)", "♪", "*sighs*".
const NOISES: &[&str] = &[
    "music",
    "laughs",
    "laughing",
    "laughter",
    "applause",
    "inaudible",
    "silence",
    "coughs",
    "coughing",
    "cough",
    "sighs",
    "sigh",
    "sighing",
    "breathing",
    "breath",
    "noise",
    "static",
    "clears throat",
    "throat clearing",
    "background noise",
    "no audio",
    "blank audio",
    "keyboard clicking",
    "typing",
    "clicking",
    "beep",
    "beeping",
    "upbeat music",
    "soft music",
    "mumbles",
    "mumbling",
    "whispers",
    "whispering",
    "indistinct",
    "indistinct chatter",
    "chuckles",
    "sniffs",
    "groans",
    "humming",
    "pause",
    "wind blowing",
    "crosstalk",
];

fn is_noise(inner: &str) -> bool {
    let t = inner.trim().trim_matches(|c: char| c == '_' || c == '-').to_lowercase().replace('_', " ");
    NOISES.contains(&t.as_str())
}

/// Whether whisper went round in a loop: the same six words twice, or the
/// same three words three times ("She passed. Cotton. She passed. Cotton.
/// She passed."). It does that when its window is cut short. People rarely
/// say either: of 900 LibriSpeech and Common Voice transcripts, one would.
pub fn loops(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let repeated = |n: usize, times: usize| {
        let mut seen = std::collections::HashMap::new();
        words.windows(n).any(|w| {
            let count = seen.entry(w).or_insert(0);
            *count += 1;
            *count >= times
        })
    };
    repeated(6, 2) || repeated(3, 3)
}

/// Text without whisper's non-speech annotations: anything in square
/// brackets, and (…) or *…* naming a sound; music notes.
pub(crate) fn strip_noises(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['[', '(', '*', '♪', '♫']) {
        out.push_str(&rest[..i]);
        let c = rest[i..].chars().next().unwrap();
        let after = &rest[i + c.len_utf8()..];
        let close = match c {
            '[' => ']',
            '(' => ')',
            '*' => '*',
            _ => {
                rest = after; // a music note
                continue;
            }
        };
        match after.find(close) {
            Some(j) if c == '[' || is_noise(&after[..j]) => rest = &after[j + 1..],
            _ => {
                out.push(c);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What whisper says over silence or noise (it was trained on subtitles).
const PHANTOMS: &[&str] = &[
    "thank you",
    "thanks",
    "thank you very much",
    "thank you so much",
    "thanks for watching",
    "thank you for watching",
    "thanks for watching and see you next time",
    "please subscribe",
    "please like and subscribe",
    "subscribe",
    "bye",
    "goodbye",
    "you",
    "so",
    "okay",
    "oh",
    "uh",
    "um",
    "hmm",
    "i'm sorry",
    "see you next time",
    "the end",
];
/// Always phantoms, however clearly "said": subtitle credits.
const CREDITS: &[&str] = &["amara.org", "subtitles by", "captions by", "transcribed by", "translated by"];

fn normalized(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' || c == '.' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('.')
        .trim()
        .to_string()
}

/// Whether a whole transcript is probably made up: a subtitle credit, or one
/// of whisper's stock phrases when there was hardly any speech
/// (`speech_s` seconds of it) or it was barely audible (`level`).
pub(crate) fn is_phantom(text: &str, speech_s: f32, level: f32) -> bool {
    let n = normalized(text);
    if n.is_empty() {
        return true;
    }
    if CREDITS.iter().any(|c| n.contains(c)) {
        return true;
    }
    let stock = PHANTOMS.contains(&n.trim_end_matches('.')) || n.split(['.', ' ']).all(|w| w.is_empty() || w == "you");
    stock && (speech_s < 0.6 || level < 0.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_are_found() {
        assert!(loops("She passed. Cotton. She passed. Cotton. She passed. Cotton. She passed."));
        assert!(loops("You must look at him in the face, fight him. You must look at him in the face."));
        assert!(loops("Thank you. Thank you. Thank you. Thank you."));
        assert!(!loops("Let's meet at 5. Wait, no, at 6."));
        assert!(!loops("I think we should ship it, I think so, on Monday."));
        assert!(!loops(""));
    }

    /// A segment `down` dB below -20 dBFS (close-up speech).
    fn seg(start: f32, end: f32, text: &str, down: f32) -> Segment {
        let db = -20.0 - down;
        Segment { start_s: start, end_s: end, text: text.into(), level: db_to_level(db), db, quiet: false }
    }

    #[test]
    fn quiet_aside_in_the_middle() {
        let mut s = vec![
            seg(0.0, 3.0, "Let's meet at four.", 0.0),
            seg(3.4, 4.6, "where did I put my coffee", 14.0),
            seg(5.0, 8.0, "And send the report first.", 1.0),
        ];
        mark_quiet(&mut s);
        assert_eq!(s.iter().map(|s| s.quiet).collect::<Vec<_>>(), [false, true, false]);
        let t = Transcript::from_segments(s);
        assert_eq!(t.text, "Let's meet at four. where did I put my coffee And send the report first.");
        assert_eq!(
            t.for_cleanup(true),
            "Let's meet at four. <quiet>where did I put my coffee</quiet> And send the report first."
        );
        assert_eq!(t.for_cleanup(false), t.text);
    }

    #[test]
    fn a_little_softer_is_not_quiet() {
        let mut s = vec![seg(0.0, 3.0, "one", 0.0), seg(3.0, 6.0, "two", 4.0), seg(6.0, 7.0, "three", 8.0)];
        mark_quiet(&mut s);
        assert!(s.iter().all(|s| !s.quiet), "{s:?}");
    }

    #[test]
    fn the_quiet_part_is_never_most_of_it() {
        // Mostly quiet speech: that's just how loud you talk, the loud bit stands out instead.
        let mut s = vec![seg(0.0, 1.0, "HEY", 0.0), seg(1.0, 6.0, "soft speech all along", 15.0)];
        mark_quiet(&mut s);
        assert!(s.iter().all(|s| !s.quiet), "{s:?}");
        // Everything equally quiet
        let mut s = vec![seg(0.0, 2.0, "a", 20.0), seg(2.0, 4.0, "b", 20.0)];
        mark_quiet(&mut s);
        assert!(s.iter().all(|s| !s.quiet));
    }

    #[test]
    fn too_little_to_compare() {
        let mut one = vec![seg(0.0, 5.0, "just one", 20.0)];
        mark_quiet(&mut one);
        assert!(!one[0].quiet);
        let mut short = vec![seg(0.0, 0.6, "hi", 0.0), seg(0.7, 1.2, "there", 20.0)];
        mark_quiet(&mut short);
        assert!(short.iter().all(|s| !s.quiet), "under 1.5 s of speech");
    }

    #[test]
    fn runs_merge() {
        let mut s = vec![
            seg(0.0, 2.0, "a", 0.0),
            seg(2.0, 3.0, "b", 0.0),
            seg(3.0, 4.0, "c", 15.0),
            seg(4.0, 5.0, "d", 15.0),
            seg(5.0, 7.0, "e", 0.0),
        ];
        mark_quiet(&mut s);
        let merged = merge_runs(s);
        let texts: Vec<_> = merged.iter().map(|s| (s.text.as_str(), s.quiet)).collect();
        assert_eq!(texts, [("a b", false), ("c d", true), ("e", false)]);
        assert_eq!((merged[1].start_s, merged[1].end_s), (3.0, 5.0));
    }

    #[test]
    fn words_to_phrases() {
        let w = |t: &str, a: f32, b: f32, span| Word { text: t.into(), start_s: a, end_s: b, span };
        let words = [
            w(" Meet", 0.0, 0.3, 0),
            w(" at", 0.3, 0.5, 0),
            w(" four.", 0.5, 0.9, 0),
            w(" Hmm", 1.0, 1.3, 0),
            w(" what", 2.0, 2.2, 0),
            w(" else", 2.2, 2.5, 0),
            w(" and", 2.5, 2.7, 1),
            w(" more", 2.7, 2.9, 1),
            w("", 2.9, 2.9, 1),
        ];
        let samples = vec![1000i16; 3 * RATE as usize];
        let p = phrases(&words, &samples, 0.5, &[(0.0, 3.0)]);
        let texts: Vec<_> = p.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Meet at four.", "Hmm", "what else", "and more"]);
        assert!(p.iter().all(|s| s.level > 0.3));
    }

    #[test]
    fn db_on_the_meter_scale() {
        // Same scale as audio::level
        let chunk = vec![1000i16; 3200];
        let db = speech_db(&chunk, 0.0, 0.2);
        assert!((db_to_level(db) - crate::audio::level(&chunk)).abs() < 1e-3);
        assert_eq!(db_to_level(FLOOR_DB), 0.0);
        assert_eq!(db_to_level(0.0), 1.0);
    }

    #[test]
    fn phrase_loudness_where_there_was_speech() {
        // Loud 0-1 s, quiet 2-3 s, silence between; whisper's timing for the
        // quiet phrase starts at 1.0, in the pause.
        let mut samples = vec![0i16; 3 * RATE as usize];
        for (i, s) in samples.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let amp = if t < 1.0 {
                8000.0
            } else if t >= 2.0 {
                800.0
            } else {
                0.0
            };
            *s = (amp * (i as f32 / 5.0).sin()) as i16;
        }
        let w = |t: &str, a: f32, b: f32, span| Word { text: t.into(), start_s: a, end_s: b, span };
        let words = [w("Loud", 0.0, 1.0, 0), w("quiet", 1.0, 2.6, 1), w("x", 2.6, 2.62, 1)];
        let p = phrases(&words, &samples, 0.5, &[(0.0, 1.0), (2.0, 3.0)]);
        assert_eq!(p.len(), 2);
        assert!((p[0].db - p[1].db - 20.0).abs() < 1.0, "{p:?}");
        // Too little speech to measure: the neighbor's loudness
        let p = phrases(&words, &samples, 0.5, &[(0.0, 1.0)]);
        assert_eq!(p[1].db, p[0].db);
    }

    #[test]
    fn speech_level_ignores_pauses() {
        let mut samples = vec![0i16; 2 * RATE as usize];
        for (i, s) in samples.iter_mut().enumerate().take(RATE as usize) {
            *s = if (i / 40) % 2 == 0 { 8000 } else { -8000 };
        }
        let half = speech_db(&samples, 0.0, 2.0); // half speech, half silence
        let full = speech_db(&samples, 0.0, 1.0);
        assert!((half - full).abs() < 0.1, "{half} vs {full}");
        assert!((full - -12.25).abs() < 0.1, "8000 of 32768: {full}");
        assert_eq!(speech_db(&samples, 1.5, 2.0), FLOOR_DB);
        assert_eq!(speech_db(&samples, 3.0, 4.0), FLOOR_DB, "past the end");
    }

    #[test]
    fn noises_stripped() {
        assert_eq!(strip_noises("[BLANK_AUDIO]"), "");
        assert_eq!(strip_noises(" Hello [Music] world. "), "Hello world.");
        assert_eq!(strip_noises("(laughs) That's funny *sighs* ok ♪"), "That's funny ok");
        assert_eq!(strip_noises("(upbeat music)"), "");
        // Real parentheses and stars stay
        assert_eq!(strip_noises("Call f(x) and (maybe) g"), "Call f(x) and (maybe) g");
        assert_eq!(strip_noises("a * b"), "a * b");
        assert_eq!(strip_noises("unclosed ( bracket"), "unclosed ( bracket");
        assert_eq!(strip_noises("[unclosed"), "[unclosed");
    }

    #[test]
    fn phantoms() {
        assert!(is_phantom("Thank you.", 0.3, 0.8));
        assert!(is_phantom(" Thank you.", 2.0, 0.1), "barely audible");
        assert!(!is_phantom("Thank you.", 1.2, 0.8), "said clearly");
        assert!(is_phantom("Subtitles by the Amara.org community", 3.0, 0.9));
        assert!(is_phantom("you you you", 0.2, 0.5));
        assert!(is_phantom("", 3.0, 0.9));
        assert!(!is_phantom("Thank you for the report, let's ship it.", 0.5, 0.5));
    }
}
