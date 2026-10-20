//! Tuning the quiet marking: loud speech, a softer aside (GAIN, after a PAUSE
//! in seconds), loud speech again, through the engine (espeak-ng's voice).
//!
//!     cargo run -p localflow-speech --release --example aside -- GAIN PAUSE [MODEL]

use localflow_speech::{Engine, RATE, SpeechConfig, read_wav};

fn say(text: &str, gain: f32) -> Vec<i16> {
    let path = std::env::temp_dir().join(format!("localflow-aside-{}.wav", std::process::id()));
    std::process::Command::new("espeak-ng")
        .args(["-v", "en-us", "-s", "165", "-w"])
        .arg(&path)
        .arg(text)
        .status()
        .unwrap();
    let s = read_wav(&std::fs::read(&path).unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    s.iter().map(|&x| (f32::from(x) * gain) as i16).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let gain: f32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(0.12);
    let pause: f32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0.8);
    let cfg = SpeechConfig { model: args.get(2).cloned(), ..Default::default() };
    let mut audio = say("Let's meet at four tomorrow to go over the launch plan and", 1.0);
    audio.extend(vec![0; (pause * RATE as f32) as usize]);
    audio.extend(say("hmm where did I put my coffee", gain));
    audio.extend(vec![0; (pause * RATE as f32) as usize]);
    audio.extend(say("bring the budget numbers.", 1.0));
    let mut engine = Engine::load(&cfg.model_file().0, true).unwrap();
    let t = engine.transcribe(&audio, &cfg.options()).unwrap();
    for s in &t.segments {
        println!(
            "  {:5.2}-{:5.2} {:6.1} dB {} {}",
            s.start_s,
            s.end_s,
            s.db,
            if s.quiet { "QUIET" } else { "     " },
            s.text
        );
    }
}
