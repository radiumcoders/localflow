//! The microphone, through the OS's audio API (ALSA/PipeWire on Linux,
//! WASAPI on Windows, Core Audio on macOS): the default or one picked by
//! name, mixed to mono at its own rate while recording, resampled to 16 kHz
//! when it stops.

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
    fallback: Option<String>,
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

/// Open `device` and start it: the stream and its rate.
fn record(device: &cpal::Device, mono: Arc<Mutex<Vec<f32>>>, on_level: OnLevel) -> Result<(cpal::Stream, u32)> {
    let supported = device.default_input_config().map_err(|e| Error(format!("couldn't open the microphone ({e})")))?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => open::<f32>(device, &config, mono, on_level)?,
        SampleFormat::I16 => open::<i16>(device, &config, mono, on_level)?,
        SampleFormat::U16 => open::<u16>(device, &config, mono, on_level)?,
        SampleFormat::I32 => open::<i32>(device, &config, mono, on_level)?,
        SampleFormat::F64 => open::<f64>(device, &config, mono, on_level)?,
        other => return err(format!("microphone sample format {other} isn't supported")),
    };
    stream.play().map_err(|e| Error(format!("couldn't start the microphone ({e})")))?;
    Ok((stream, config.sample_rate.0))
}

/// Which microphone to record.
#[derive(Debug, PartialEq)]
enum Pick<D> {
    /// The system's default: no other was asked for.
    Default,
    /// The one asked for.
    Named(D),
    /// The one asked for isn't there (a headset unplugged): the default instead.
    Missing,
}

/// The device named `wanted` among `devices` (name, device). It stops at the
/// first match: listing a device can mean opening it.
fn pick<D>(wanted: Option<&str>, devices: impl IntoIterator<Item = (String, D)>) -> Pick<D> {
    let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) else { return Pick::Default };
    devices.into_iter().find(|(name, _)| name.trim() == wanted).map_or(Pick::Missing, |(_, d)| Pick::Named(d))
}

/// Start the microphone named `mic`, else the default: the stream, its rate,
/// and why it's the default when another was asked for. A missing or busy
/// microphone never stops a dictation.
fn start_mic(mic: Option<&str>, mono: Arc<Mutex<Vec<f32>>>, on_level: OnLevel)
    -> Result<(cpal::Stream, u32, Option<String>)> {
    let host = cpal::default_host();
    let devices = host.input_devices().into_iter().flatten().map(|d| (d.name().unwrap_or_default(), d));
    let fallback = match pick(mic, devices) {
        Pick::Default => None,
        Pick::Named(device) => match record(&device, mono.clone(), on_level.clone()) {
            Ok((stream, rate)) => return Ok((stream, rate, None)),
            // Another app has it, or it went away just now.
            Err(e) => Some(format!("{}: {e}", mic.unwrap_or_default())),
        },
        // Unplugged; on Linux, also a sound card another program has open
        // (PipeWire keeps one open for a few seconds after it's used).
        Pick::Missing => Some(format!("{} isn't available", mic.unwrap_or_default())),
    };
    let device = host.default_input_device().ok_or_else(|| Error("no microphone".into()))?;
    let (stream, rate) = record(&device, mono, on_level)?;
    Ok((stream, rate, fallback))
}

/// Whether a microphone is offered for picking. On Linux, cpal's ALSA host
/// lists the sound servers it can reach (default, pipewire, pulse, jack,
/// oss), then each sound card by its name ("HD-Audio Generic", a USB
/// headset's). default, pipewire and pulse record what the desktop's sound
/// settings chose, so they stay; jack and oss aren't microphones a person
/// picks (JACK's inputs are wired up in JACK, OSS is an old emulation). A
/// card is recorded directly, past PipeWire, and is listed only while no
/// other program has it open. macOS and Windows list only real devices.
fn offered(name: &str) -> bool {
    !(cfg!(target_os = "linux") && matches!(name, "jack" | "oss"))
}

/// The microphones there are, by name, for `LOCALFLOW_MIC`. Two of the same
/// model have the same name, so it's listed once (and the first is recorded).
pub fn microphones() -> Vec<String> {
    let mut names: Vec<String> = vec![];
    let devices = cpal::default_host().input_devices().into_iter().flatten();
    for name in devices.filter_map(|d| d.name().ok()) {
        if offered(&name) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The system's default microphone's name ("default" on Linux, where the
/// sound server picks the device).
pub fn default_microphone() -> Option<String> {
    cpal::default_host().default_input_device().and_then(|d| d.name().ok())
}

impl Recorder {
    /// Start recording the default microphone. `on_level` gets the loudness
    /// (0..1) of each chunk as it arrives, for the level meter.
    pub fn start(on_level: impl Fn(f32) + Send + Sync + 'static) -> Result<Recorder> {
        Recorder::start_on(None, on_level)
    }

    /// Start recording the microphone named `mic` (see [`microphones`]), or
    /// the default for None. When it isn't there or won't open, the
    /// default is recorded instead and [`Recorder::fallback`] says why.
    pub fn start_on(mic: Option<&str>, on_level: impl Fn(f32) + Send + Sync + 'static) -> Result<Recorder> {
        let mono = Arc::new(Mutex::new(Vec::new()));
        let on_level: OnLevel = Arc::new(on_level);
        let (started, ready) = mpsc::channel();
        let (stop, stopped) = mpsc::channel::<()>();
        let samples = mono.clone();
        let mic = mic.map(String::from);
        let thread = std::thread::spawn(move || match start_mic(mic.as_deref(), samples, on_level) {
            Ok((stream, rate, fallback)) => {
                let _ = started.send(Ok((rate, fallback)));
                let _ = stopped.recv(); // until stop(), or the Recorder is dropped
                drop(stream);
            }
            Err(e) => {
                let _ = started.send(Err(e));
            }
        });
        let (rate, fallback) = ready.recv().map_err(|_| Error("the microphone thread stopped".into()))??;
        Ok(Recorder { stop, thread, mono, rate, fallback })
    }

    /// Why the default microphone is recorded instead of the one asked for
    /// ("Jabra Evolve2 65 isn't available"); None when it's the one asked for.
    pub fn fallback(&self) -> Option<&str> {
        self.fallback.as_deref()
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
    use super::{Pick, offered, pick};

    fn devices(names: &[&str]) -> Vec<(String, usize)> {
        names.iter().enumerate().map(|(i, n)| (n.to_string(), i)).collect()
    }

    #[test]
    fn picks_the_named_microphone() {
        let there = devices(&["MacBook Pro Microphone", "Jabra Evolve2 65"]);
        assert_eq!(pick(None, there.clone()), Pick::Default);
        assert_eq!(pick(Some(" "), there.clone()), Pick::Default);
        assert_eq!(pick(Some("Jabra Evolve2 65"), there.clone()), Pick::Named(1));
        assert_eq!(pick(Some(" Jabra Evolve2 65 "), there.clone()), Pick::Named(1));
        // Unplugged: the default, never no dictation.
        assert_eq!(pick(Some("AirPods Pro"), there.clone()), Pick::Missing);
        assert_eq!(pick(Some("jabra evolve2 65"), there), Pick::Missing); // names are exact
        assert_eq!(pick(Some("Jabra Evolve2 65"), devices(&[])), Pick::Missing);
    }

    #[test]
    fn stops_looking_once_found() {
        let more = std::iter::from_fn(|| -> Option<(String, usize)> { panic!("listed past the match") });
        assert_eq!(pick(Some("pipewire"), devices(&["default", "pipewire"]).into_iter().chain(more)), Pick::Named(1));
        let none = std::iter::from_fn(|| -> Option<(String, usize)> { panic!("listed for the default") });
        assert_eq!(pick(None, none), Pick::Default);
    }

    #[test]
    fn offers_microphones() {
        for name in ["default", "pipewire", "pulse", "HD-Audio Generic", "Jabra Evolve2 65"] {
            assert!(offered(name), "{name}");
        }
        assert_eq!(offered("jack"), !cfg!(target_os = "linux"));
        assert_eq!(offered("oss"), !cfg!(target_os = "linux"));
    }

    /// The session hands its recorder to a worker thread; Core Audio's
    /// stream alone couldn't go (this failed to build on macOS).
    #[test]
    fn recorder_is_send() {
        fn send<T: Send>() {}
        send::<super::Recorder>();
    }
}
