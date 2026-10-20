//! Samples: loudness for the level meter, resampling to 16 kHz, and WAV files.

use std::f64::consts::PI;

use crate::{Result, err};

/// 16 kHz mono 16-bit: what Handy's --transcribe-file takes.
pub const RATE: u32 = 16000;

/// Loudness of a chunk, 0..1 (RMS on a log scale; speech lands around 0.5-0.9).
pub fn level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
    let rms = (sum / samples.len() as f64).sqrt();
    ((20.0 * rms.max(1.0).log10() - 25.0) / 45.0).clamp(0.0, 1.0) as f32
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 { 1.0 } else { (PI * x).sin() / (PI * x) }
}

/// Band-limited resampling (Hann-windowed sinc), so a 48 kHz microphone
/// doesn't fold noise above 8 kHz into the speech band at 16 kHz.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let cutoff = (1.0 / ratio).min(1.0); // of the input's Nyquist frequency
    let half = (16.0 * ratio.max(1.0)).ceil() as isize; // window, in input samples each side
    let n_out = (input.len() as f64 / ratio).floor() as usize;
    (0..n_out)
        .map(|n| {
            let t = n as f64 * ratio;
            let center = t.floor() as isize;
            let (mut acc, mut weights) = (0.0, 0.0);
            for k in (center - half + 1).max(0)..=(center + half).min(input.len() as isize - 1) {
                let x = t - k as f64;
                let window = 0.5 * (1.0 + (PI * x / half as f64).cos());
                let w = cutoff * sinc(cutoff * x) * window;
                acc += w * f64::from(input[k as usize]);
                weights += w;
            }
            (if weights == 0.0 { 0.0 } else { acc / weights }) as f32
        })
        .collect()
}

/// A 16-bit mono WAV file at `rate`.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A 16-bit PCM WAV file as 16 kHz mono (channels mixed, resampled).
pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return err("not a WAV file");
    }
    let (mut pos, mut format, mut samples) = (12, None, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            format = Some((u16_at(0), u16_at(2), rate, u16_at(14)));
        } else if id == b"data" {
            samples = Some(body);
        }
        pos += 8 + size + size % 2;
    }
    let (Some((tag, channels, rate, bits)), Some(data)) = (format, samples) else {
        return err("WAV file without format or data");
    };
    if tag != 1 || bits != 16 || channels == 0 {
        return err(format!("WAV file isn't 16-bit PCM (format {tag}, {bits} bits)"));
    }
    let frames: Vec<f32> = data
        .chunks_exact(2 * channels as usize)
        .map(|frame| {
            let sum: f32 = frame.as_chunks::<2>().0.iter().map(|&s| f32::from(i16::from_le_bytes(s))).sum();
            sum / f32::from(channels)
        })
        .collect();
    Ok(resample(&frames, rate, RATE).into_iter().map(|s| s.round().clamp(-32768.0, 32767.0) as i16).collect())
}
