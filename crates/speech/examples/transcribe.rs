//! A WAV file through the speech engine, with timings and each segment's
//! loudness (quiet ones marked): for trying models and tuning.
//!
//!     cargo run -p localflow-speech --release --example transcribe -- talk.wav [MODEL] [--cpu] [--no-vad]
//!
//! MODEL is an id (tiny.en, base.en, small.en, large-v3-turbo, base, small) or a
//! ggml file; it's downloaded first if needed, with the voice detector.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use localflow_speech::{Engine, SpeechConfig, models, read_wav};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(wav) = positional.first() else {
        eprintln!("usage: transcribe FILE.wav [MODEL] [--cpu] [--no-vad]");
        std::process::exit(2);
    };
    let cfg = SpeechConfig {
        model: positional.get(1).map(|s| s.to_string()),
        use_gpu: !flag("--cpu"),
        vocabulary: std::env::var("VOCAB").map(|v| v.split(',').map(String::from).collect()).unwrap_or_default(),
        ..Default::default()
    };
    let (path, model) = cfg.model_file();
    let cancel = AtomicBool::new(false);
    for m in model.into_iter().chain((!flag("--no-vad")).then_some(&models::VAD)) {
        if !m.installed() {
            eprintln!("downloading {} ({} MB)", m.name, m.size_mb());
            let mut last = 0;
            models::download(
                m.id,
                &mut |done, total| {
                    let pct = done * 100 / total.max(1);
                    if pct >= last + 10 {
                        eprintln!("  {pct}%");
                        last = pct;
                    }
                },
                &cancel,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
    }
    let samples = read_wav(&std::fs::read(wav).expect("can't read the WAV")).expect("not a 16-bit WAV");
    let start = Instant::now();
    let mut engine = Engine::load(&path, cfg.use_gpu).unwrap_or_else(|e| panic!("{e}"));
    if flag("--no-vad") {
        engine.disable_vad();
    }
    let loaded = start.elapsed();
    let start = Instant::now();
    let t = engine.transcribe(&samples, &cfg.options()).unwrap_or_else(|e| panic!("{e}"));
    let took = start.elapsed();
    eprintln!(
        "{}: {:.1}s of audio, load {} ms, transcribe {} ms, vad {}",
        path.file_name().unwrap().to_string_lossy(),
        samples.len() as f32 / 16000.0,
        loaded.as_millis(),
        took.as_millis(),
        engine.has_vad()
    );
    for s in &t.segments {
        eprintln!(
            "  {:6.2}-{:6.2}  {:6.1} dB{}  {}",
            s.start_s,
            s.end_s,
            s.db,
            if s.quiet { " QUIET" } else { "      " },
            s.text
        );
    }
    println!("{}", t.for_cleanup(true));
}
