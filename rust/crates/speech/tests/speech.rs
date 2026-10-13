//! Audio handling and transcription, without a microphone or Handy.

use std::f32::consts::PI;

use localflow_speech::{RATE, level, read_wav, resample, wav_bytes};

fn sine(freq: f32, rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
    (0..(rate as f32 * seconds) as usize).map(|i| amplitude * (2.0 * PI * freq * i as f32 / rate as f32).sin()).collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn level_matches_the_python_meter() {
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

#[cfg(unix)]
#[test]
fn transcribes_with_handy() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("localflow-handy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("handy");
    // A stand-in for Handy: checks its arguments and the WAV, answers like --json does.
    std::fs::write(&fake, "#!/bin/sh\n\
        [ \"$1\" = --transcribe-file ] && [ \"$3\" = --json ] && [ \"$5\" = tiny-model ] || exit 3\n\
        head -c 4 \"$2\" | grep -q RIFF || exit 4\n\
        echo '{\"text\": \" call mom tomorrow \"}'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: the only test in this binary that reads or writes the environment.
    unsafe { std::env::set_var("DICTATE_HANDY", &fake) };
    assert_eq!(localflow_speech::handy_path(), Some(fake.clone()));
    assert_eq!(localflow_speech::transcribe(&[0; 1600], Some("tiny-model")).unwrap(), "call mom tomorrow");
    let e = localflow_speech::transcribe(&[0; 1600], Some("other")).unwrap_err();
    assert!(e.0.starts_with("Handy couldn't transcribe"), "{e}");
}
