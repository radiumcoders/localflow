//! Audio handling and transcription, without a microphone or Handy.

use std::f32::consts::PI;

use localflow_speech::{Engine, RATE, SpeechConfig, level, models, read_wav, resample, wav_bytes};

fn sine(freq: f32, rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
    (0..(rate as f32 * seconds) as usize).map(|i| amplitude * (2.0 * PI * freq * i as f32 / rate as f32).sin()).collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn level_meter_scale() {
    assert_eq!(level(&[]), 0.0);
    assert_eq!(level(&[0; 1600]), 0.0);
    // RMS 1000: (20 * log10(1000) - 25) / 45
    assert!((level(&[1000, -1000, 1000, -1000]) - 35.0 / 45.0).abs() < 1e-6);
    assert_eq!(level(&[32767, -32768]), 1.0);
}

#[test]
fn resampling_keeps_speech_and_drops_what_would_alias() {
    // 1 kHz (speech band) from 48 kHz to 16 kHz: same tone, same loudness.
    let out = resample(&sine(1000.0, 48000, 1.0, 10000.0), 48000, RATE);
    assert_eq!(out.len(), 16000);
    let want = sine(1000.0, RATE, 1.0, 10000.0);
    let err: Vec<f32> = out.iter().zip(&want).skip(100).take(15800).map(|(a, b)| a - b).collect();
    assert!(rms(&err) < 0.02 * 10000.0, "error rms {}", rms(&err));
    // 12 kHz can't exist at 16 kHz; without filtering it would fold to 4 kHz.
    let folded = resample(&sine(12000.0, 48000, 1.0, 10000.0), 48000, RATE);
    assert!(rms(&folded[100..15900]) < 0.05 * rms(&sine(12000.0, 48000, 1.0, 10000.0)), "{}", rms(&folded));
    // Other rates (44.1 kHz) and upsampling work too.
    assert_eq!(resample(&vec![0.0; 44100], 44100, RATE).len(), 16000);
    assert_eq!(resample(&vec![0.0; 8000], 8000, RATE).len(), 16000);
}

#[test]
fn wav_round_trip() {
    let samples: Vec<i16> = (0..1600).map(|i| ((i * 37) % 2000 - 1000) as i16).collect();
    let bytes = wav_bytes(&samples, RATE);
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(bytes.len(), 44 + samples.len() * 2);
    assert_eq!(read_wav(&bytes).unwrap(), samples);
}

#[test]
fn stereo_48k_wav_becomes_16k_mono() {
    // A stereo 48 kHz file: header written by hand, the left channel a tone, the right silent.
    let tone: Vec<i16> = sine(440.0, 48000, 0.5, 8000.0).iter().map(|&s| s as i16).collect();
    let mut data = Vec::new();
    for s in &tone {
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 2, 0], 48000u32.to_le_bytes(), (48000u32 * 4).to_le_bytes(), [4, 0, 16, 0]] {
        wav.extend_from_slice(&v);
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    let out = read_wav(&wav).unwrap();
    assert_eq!(out.len(), 8000);
    let peak = out[100..7900].iter().map(|s| s.unsigned_abs()).max().unwrap();
    assert!((3800..4200).contains(&peak), "mixed to mono at half the level: {peak}");
    assert!(read_wav(b"not a wav file").is_err());
}

/// Speech from espeak-ng, as 16 kHz mono at `gain` (1.0: as espeak makes it).
fn say(text: &str, gain: f32) -> Option<Vec<i16>> {
    let path = std::env::temp_dir().join(format!("localflow-say-{}-{}.wav", std::process::id(), text.len()));
    let ok = std::process::Command::new("espeak-ng")
        .args(["-v", "en-us", "-s", "165", "-w"])
        .arg(&path)
        .arg(text)
        .status()
        .is_ok_and(|s| s.success());
    let wav = std::fs::read(&path).ok();
    let _ = std::fs::remove_file(&path);
    if !ok {
        return None;
    }
    let samples = read_wav(&wav?).ok()?;
    Some(
        samples
            .iter()
            .map(|&s| (f32::from(s) * gain).clamp(-32768.0, 32767.0) as i16)
            .collect(),
    )
}

fn silence(seconds: f32) -> Vec<i16> {
    vec![0; (seconds * RATE as f32) as usize]
}

/// With a real model: LOCALFLOW_TEST_MODEL=small.en cargo test -p localflow-speech --release
/// (downloads the model and the voice detector if they aren't there; needs espeak-ng).
#[test]
fn transcribes_with_whisper() {
    let Ok(id) = std::env::var("LOCALFLOW_TEST_MODEL") else {
        eprintln!("skipped: set LOCALFLOW_TEST_MODEL to a model id to run it");
        return;
    };
    let Some(speech) = say("Please send the quarterly report to Sarah by Friday.", 1.0) else {
        eprintln!("skipped: no espeak-ng");
        return;
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    for m in [id.as_str(), "silero-vad"] {
        models::download(m, &mut |_, _| {}, &cancel).unwrap();
    }
    let cfg = SpeechConfig {
        model: Some(id.clone()),
        use_gpu: false,
        ..Default::default()
    };
    let mut engine = Engine::load(&cfg.model_file().0, false).unwrap();
    assert!(engine.has_vad());

    let mut audio = silence(1.0);
    audio.extend(&speech);
    audio.extend(silence(1.0));
    let t = engine.transcribe(&audio, &cfg.options()).unwrap();
    let words = t.text.to_lowercase();
    for w in ["send", "quarterly", "report", "friday"] {
        assert!(words.contains(w), "{w} in {:?}", t.text);
    }
    assert!(!t.has_quiet(), "{t:?}");
    let first = &t.segments[0];
    assert!(
        (0.7..1.6).contains(&first.start_s),
        "timing back in the recording's time: {first:?}"
    );

    // Nothing said: nothing heard (not "Thank you.")
    let t = engine.transcribe(&silence(3.0), &cfg.options()).unwrap();
    assert!(t.is_empty(), "{t:?}");
    let noise: Vec<i16> = (0..3 * RATE as i32).map(|i| ((i * 7919) % 401 - 200) as i16).collect();
    let t = engine.transcribe(&noise, &cfg.options()).unwrap();
    assert!(t.is_empty(), "faint noise: {t:?}");

    // A muttered aside between two loud sentences is marked quiet
    let mut audio = say("Let's meet at four tomorrow to go over the launch plan.", 1.0).unwrap();
    audio.extend(silence(0.8));
    audio.extend(say("where did I put my coffee", 0.12).unwrap()); // about 18 dB down
    audio.extend(silence(0.8));
    audio.extend(say("And please bring the budget numbers.", 1.0).unwrap());
    let t = engine.transcribe(&audio, &cfg.options()).unwrap();
    let marked = t.for_cleanup(true);
    eprintln!("{marked}");
    let quiet: Vec<_> = t
        .segments
        .iter()
        .filter(|s| s.quiet)
        .map(|s| s.text.to_lowercase())
        .collect();
    assert_eq!(quiet.len(), 1, "{t:?}");
    assert!(quiet[0].contains("coffee"), "{quiet:?}");
    assert!(marked.contains("<quiet>") && marked.to_lowercase().contains("budget"));
}
