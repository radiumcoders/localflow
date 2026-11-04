//! Speech models side by side on LibriSpeech: word error rate, how long each
//! dictation takes, and how many transcripts went round in a loop. For
//! choosing models and their defaults on a machine, not a test.
//!
//!     cargo run -p localflow-speech --release --example bench -- \
//!         [--models tiny.en,small.en,large-v3-turbo] [--set clean|other] [--n 100] \
//!         [--modes gpu,cpu,gpu-short,cpu-full] [--out results.jsonl]
//!
//! Modes: `gpu` and `cpu` are what localflow does there (the GPU with whisper's
//! full window, the CPU with a shortened one and a second pass when it loops);
//! `gpu-short` and `cpu-full` swap the window, for comparison. The clips
//! (LibriSpeech test, CC BY 4.0) download once into the temp folder, spread
//! over the speakers. The scoring lowercases and drops punctuation, like
//! Whisper's normalizer but simpler, so its error rates run a little higher
//! than published ones; the models compare fairly.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use localflow_speech::{Engine, Options, gpu, loops, models, resample};

struct Clip {
    id: String,
    samples: Vec<i16>,
    reference: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let list = |name: &str, default: &str| -> Vec<String> {
        arg(name).unwrap_or_else(|| default.into()).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    let models_wanted = list("--models", "tiny.en,base.en,small.en,large-v3-turbo");
    let modes = list("--modes", "gpu,cpu");
    let set = arg("--set").unwrap_or_else(|| "clean".into());
    let n: usize = arg("--n").and_then(|v| v.parse().ok()).unwrap_or(100);
    let mut out = arg("--out").map(|p| fs::File::create(&p).unwrap_or_else(|e| panic!("{p}: {e}")));

    println!("machine: {} {}, GPU: {}", std::env::consts::OS, std::env::consts::ARCH, match gpu() {
        Some(g) => format!("{}{}", g.name, if g.integrated { " (integrated)" } else { "" }),
        None => "none in this build".into(),
    });
    let clips = clips(&set, n);
    let audio_s: f32 = clips.iter().map(|c| c.samples.len() as f32 / 16_000.0).sum();
    println!("LibriSpeech test-{set}: {} clips, {:.1} min\n", clips.len(), audio_s / 60.0);
    println!("| model | mode | WER % | loops | median s | p90 s | 1 s of audio takes | load s |");
    println!("|---|---|---|---|---|---|---|---|");

    for id in &models_wanted {
        let Some(model) = models::find(id) else {
            eprintln!("no model {id} (see `localflow models`)");
            continue;
        };
        for m in [model, &models::VAD] {
            if !m.installed() {
                eprintln!("downloading {} ({} MB)", m.name, m.size_mb());
                models::download(m.id, &mut |_, _| {}, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("{}: {e}", m.id));
            }
        }
        for mode in &modes {
            let (use_gpu, full_window) = match mode.as_str() {
                "gpu" => (true, None),
                "cpu" => (false, None),
                "gpu-short" => (true, Some(false)),
                "cpu-full" => (false, Some(true)),
                other => panic!("no mode {other}: gpu, cpu, gpu-short or cpu-full"),
            };
            if use_gpu && gpu().is_none() {
                continue;
            }
            let started = Instant::now();
            let mut engine = Engine::load(&model.path(), use_gpu).unwrap_or_else(|e| panic!("{id}: {e}"));
            let load_s = started.elapsed().as_secs_f32();
            let opts = Options { full_window, ..Default::default() };
            // One dictation first: the GPU's first run sets itself up.
            let _ = engine.transcribe(&clips[0].samples, &opts);
            let (mut errors, mut words, mut looped, mut times) = (0, 0, 0, Vec::new());
            for clip in &clips {
                let started = Instant::now();
                let heard = engine.transcribe(&clip.samples, &opts).map(|t| t.text).unwrap_or_else(|e| format!("<error: {e}>"));
                times.push(started.elapsed().as_secs_f32());
                let (r, h) = (normalize(&clip.reference), normalize(&heard));
                errors += distance(&r, &h);
                words += r.len();
                looped += loops(&heard) as usize;
                if let Some(f) = out.as_mut() {
                    let line = serde_json::json!({ "model": id, "mode": mode, "id": clip.id, "ref": clip.reference, "hyp": heard, "s": times.last() });
                    writeln!(f, "{line}").ok();
                }
            }
            let total: f32 = times.iter().sum();
            times.sort_by(f32::total_cmp);
            println!(
                "| {id} | {mode} | {:.2} | {looped} | {:.2} | {:.2} | {:.3} s | {load_s:.2} |",
                errors as f32 * 100.0 / words.max(1) as f32,
                times[times.len() / 2],
                times[times.len() * 9 / 10],
                total / audio_s,
            );
        }
    }
}

/// `n` clips of the set, in groups of ten spread over the whole set (so over
/// its speakers), cached in the temp folder.
fn clips(set: &str, n: usize) -> Vec<Clip> {
    let dir = std::env::temp_dir().join("localflow-bench").join(set);
    fs::create_dir_all(&dir).expect("bench folder");
    let api = |offset: usize, length: usize| -> serde_json::Value {
        let url = format!(
            "https://datasets-server.huggingface.co/rows?dataset=openslr/librispeech_asr&config={set}&split=test&offset={offset}&length={length}"
        );
        serde_json::from_reader(ureq::get(&url).call().unwrap_or_else(|e| panic!("{url}: {e}")).into_reader()).expect("rows")
    };
    let total = api(0, 1)["num_rows_total"].as_u64().expect("num_rows_total") as usize;
    let groups = n.div_ceil(10);
    let mut clips = Vec::new();
    for g in 0..groups {
        let length = 10.min(n - clips.len());
        let offset = g * (total / groups);
        let index = dir.join(format!("{offset}-{length}.tsv"));
        if !index.exists() {
            let mut lines = String::new();
            for row in api(offset, length)["rows"].as_array().expect("rows") {
                let row = &row["row"];
                let id = row["id"].as_str().expect("id");
                let src = row["audio"][0]["src"].as_str().expect("audio");
                let mut flac = Vec::new();
                ureq::get(src).call().unwrap_or_else(|e| panic!("{id}: {e}")).into_reader().read_to_end(&mut flac).expect("audio");
                fs::write(dir.join(format!("{id}.flac")), flac).expect("save");
                lines.push_str(&format!("{id}\t{}\n", row["text"].as_str().expect("text")));
            }
            fs::write(&index, lines).expect("index");
        }
        for line in fs::read_to_string(&index).expect("index").lines() {
            let (id, reference) = line.split_once('\t').expect("id\ttext");
            clips.push(Clip { id: id.into(), samples: decode(&dir.join(format!("{id}.flac"))), reference: reference.into() });
        }
    }
    clips
}

fn decode(path: &Path) -> Vec<i16> {
    let mut reader = claxon::FlacReader::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let info = reader.streaminfo();
    let shift = info.bits_per_sample.saturating_sub(16);
    let samples: Vec<i16> = reader.samples().step_by(info.channels as usize).map(|s| (s.expect("flac") >> shift) as i16).collect();
    if info.sample_rate == 16_000 {
        return samples;
    }
    let f: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
    resample(&f, info.sample_rate, 16_000).iter().map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16).collect()
}

/// Words to compare: lowercase, no punctuation, numbers and common
/// abbreviations as LibriSpeech writes them.
fn normalize(text: &str) -> Vec<String> {
    let text = text.to_lowercase().replace(['-', '—', '/'], " ");
    let mut words = Vec::new();
    for w in text.split_whitespace() {
        let w: String = w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect();
        let w = w.trim_matches('\'');
        match w {
            "" => {}
            "mr" => words.push("mister".into()),
            "mrs" => words.push("missus".into()),
            "dr" => words.push("doctor".into()),
            "st" => words.push("saint".into()),
            _ if w.chars().all(|c| c.is_ascii_digit()) => match w.parse::<u32>() {
                Ok(n) => words.extend(number(n).split_whitespace().map(String::from)),
                Err(_) => words.push(w.into()),
            },
            _ => words.push(w.into()),
        }
    }
    words
}

fn number(n: u32) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
        "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    match n {
        0..20 => ONES[n as usize].into(),
        20..100 => format!("{} {}", TENS[n as usize / 10], if !n.is_multiple_of(10) { ONES[n as usize % 10] } else { "" }).trim().into(),
        // years as said: 1922 is nineteen twenty two
        1100..2000 if !n.is_multiple_of(100) => format!("{} {}", number(n / 100), number(n % 100)),
        100..1000 => format!("{} hundred {}", ONES[n as usize / 100], if !n.is_multiple_of(100) { number(n % 100) } else { String::new() }).trim().into(),
        1000..1_000_000 => format!("{} thousand {}", number(n / 1000), if !n.is_multiple_of(1000) { number(n % 1000) } else { String::new() }).trim().into(),
        _ => n.to_string(),
    }
}

/// Word-level edit distance.
fn distance(a: &[String], b: &[String]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(x != y)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}
