//! The default microphone, through the OS's audio API (ALSA/PipeWire on
//! Linux, WASAPI on Windows, Core Audio on macOS): mixed to mono at its own
//! rate while recording, resampled to 16 kHz when it stops.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::audio::{RATE, level, resample};
use crate::{Error, Result, err};

/// Recording, until `stop`. The audio stream lives on a thread of its own
/// (Core Audio's can't move between threads), so a Recorder can.
pub struct Recorder {
    stop: Sender<()>,
    thread: JoinHandle<()>,
    mono: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

type OnLevel = Arc<dyn Fn(f32) + Send + Sync>;

fn open<T>(device: &cpal::Device, config: &cpal::StreamConfig, mono: Arc<Mutex<Vec<f32>>>, on_level: OnLevel)
    -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Mixed to mono, scaled to i16's range (what the engine and level() expect).
                let chunk: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / frame.len() as f32 * 32767.0)
                    .collect();
                let as_i16: Vec<i16> = chunk.iter().map(|&s| s.clamp(-32768.0, 32767.0) as i16).collect();
                on_level(level(&as_i16));
                mono.lock().unwrap().extend_from_slice(&chunk);
            },
            |e| eprintln!("microphone: {e}"),
            None,
        )
        .map_err(|e| Error(format!("couldn't open the microphone ({e})")))
}

/// Open the default microphone and start it: the stream and its rate.
fn record(mono: Arc<Mutex<Vec<f32>>>, on_level: OnLevel) -> Result<(cpal::Stream, u32)> {
    let device = cpal::default_host().default_input_device().ok_or_else(|| Error("no microphone".into()))?;
    let supported = device.default_input_config().map_err(|e| Error(format!("couldn't open the microphone ({e})")))?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => open::<f32>(&device, &config, mono, on_level)?,
        SampleFormat::I16 => open::<i16>(&device, &config, mono, on_level)?,
        SampleFormat::U16 => open::<u16>(&device, &config, mono, on_level)?,
        SampleFormat::I32 => open::<i32>(&device, &config, mono, on_level)?,
        SampleFormat::F64 => open::<f64>(&device, &config, mono, on_level)?,
        other => return err(format!("microphone sample format {other} isn't supported")),
    };
    stream.play().map_err(|e| Error(format!("couldn't start the microphone ({e})")))?;
    Ok((stream, config.sample_rate.0))
}

impl Recorder {
    /// Start recording the default microphone. `on_level` gets the loudness
    /// (0..1) of each chunk as it arrives, for the level meter.
    pub fn start(on_level: impl Fn(f32) + Send + Sync + 'static) -> Result<Recorder> {
        let mono = Arc::new(Mutex::new(Vec::new()));
        let on_level: OnLevel = Arc::new(on_level);
        let (started, ready) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let samples = mono.clone();
        let thread = std::thread::spawn(move || match record(samples, on_level) {
            Ok((stream, rate)) => {
                let _ = started.send(Ok(rate));
                let _ = stopped.recv(); // until stop(), or the Recorder is dropped
                drop(stream);
            }
            Err(e) => {
                let _ = started.send(Err(e));
            }
        });
        let rate = ready.recv().map_err(|_| Error("the microphone thread stopped".into()))??;
        Ok(Recorder { stop, thread, mono, rate })
    }

    /// Stop, and return what was recorded as 16 kHz mono.
    pub fn stop(self) -> Vec<i16> {
        let _ = self.stop.send(());
        let _ = self.thread.join();
        let mono = std::mem::take(&mut *self.mono.lock().unwrap());
        resample(&mono, self.rate, RATE).into_iter().map(|s| s.round().clamp(-32768.0, 32767.0) as i16).collect()
    }
}

#[cfg(test)]
mod tests {
    /// The session hands its recorder to a worker thread; Core Audio's
    /// stream alone couldn't go (this failed to build on macOS).
    #[test]
    fn recorder_is_send() {
        fn send<T: Send>() {}
        send::<super::Recorder>();
    }
}
