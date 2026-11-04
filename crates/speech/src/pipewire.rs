//! PipeWire's microphones on Linux: its audio sources, the ones the
//! desktop's sound settings show, listed by their description and recorded
//! through PipeWire, so a microphone is never busy and the sound server
//! stays in charge of the device.
//!
//! Listing runs `pw-dump`, which ships with PipeWire and prints JSON;
//! `wpctl` and `pw-cli` print text meant for people. Recording opens the ALSA
//! PCM pipewire-alsa defines with the source as its target,
//! `pipewire:NODE=<node.name>`. cpal opens only the devices it lists, so the
//! PCM is opened with the alsa crate (cpal's own, on the libasound localflow
//! links already). The target is the device's argument, not an environment
//! variable, so nothing changes the process's environment, and nothing links
//! libpipewire: without PipeWire, pw-dump isn't there or can't connect, and
//! localflow lists cpal's devices instead. A `pw-record` process per
//! dictation would work too, but its first audio came ~45 ms after it
//! started, and it's one more program to find.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};

use crate::audio::{RATE, level};
use crate::recorder::{Microphone, OnLevel};
use crate::{Error, Result};

/// 20 ms at 16 kHz: how often the level meter moves.
const PERIOD: usize = 320;

/// The longest a read waits for audio, so a stop is seen that soon even when
/// the source sends nothing (a Bluetooth headset dropping out).
const WAIT_MS: u32 = 100;

/// How long pw-dump may take (it takes ~20 ms): a PipeWire that doesn't
/// answer is no PipeWire, rather than a dictation or a window that hangs.
const PW_DUMP_DEADLINE: Duration = Duration::from_secs(1);

/// PipeWire's audio sources, or None when PipeWire isn't running or doesn't
/// answer (or pw-dump isn't installed).
pub fn sources() -> Option<Vec<Microphone>> {
    let out = output_within(Command::new("pw-dump").stdin(Stdio::null()), PW_DUMP_DEADLINE)?;
    parse(&String::from_utf8_lossy(&out))
}

/// What `cmd` prints, when it succeeds within `deadline`; else it's killed.
fn output_within(cmd: &mut Command, deadline: Duration) -> Option<Vec<u8>> {
    let start = Instant::now();
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread of its own: a full pipe would keep the child from exiting.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = vec![];
        let _ = stdout.read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(deadline).ok();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return out.filter(|_| status.success()),
            Ok(None) if out.is_some() && start.elapsed() < deadline => std::thread::sleep(Duration::from_millis(1)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// pw-dump's objects: each node of class Audio/Source (a microphone; an
/// output's monitor is a sink, and virtual sources say so in their class),
/// with the default from the "default" metadata.
fn parse(json: &str) -> Option<Vec<Microphone>> {
    let objects: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let default = objects
        .iter()
        .filter_map(|o| o["metadata"].as_array())
        .flatten()
        .find(|m| m["key"] == "default.audio.source")
        .map(|m| match &m["value"] {
            // An object, or a JSON string holding one.
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
            value => value.clone(),
        })
        .and_then(|v| v["name"].as_str().map(String::from));
    let sources = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Node")
        .map(|o| &o["info"]["props"])
        .filter(|props| props["media.class"] == "Audio/Source")
        .filter_map(|props| {
            let id = props["node.name"].as_str()?;
            let name = props["node.description"].as_str().or(props["node.nick"].as_str()).unwrap_or(id);
            Some(Microphone { id: id.into(), name: name.into(), default: default.as_deref() == Some(id) })
        })
        .collect();
    Some(sources)
}

/// Open the source `node` through PipeWire, as 16 kHz mono. Non-blocking:
/// `capture` waits for audio itself, for a while at most.
pub fn open(node: &str) -> Result<PCM> {
    let failed = |e: alsa::Error| Error(format!("couldn't open the microphone ({e})"));
    let pcm = PCM::new(&format!("pipewire:NODE={node}"), Direction::Capture, true).map_err(failed)?;
    {
        let hw = HwParams::any(&pcm).map_err(failed)?;
        hw.set_access(Access::RWInterleaved).map_err(failed)?;
        hw.set_format(Format::s16()).map_err(failed)?;
        hw.set_channels(1).map_err(failed)?;
        hw.set_rate(RATE, ValueOr::Nearest).map_err(failed)?;
        hw.set_period_size_near(PERIOD as alsa::pcm::Frames, ValueOr::Nearest).map_err(failed)?;
        hw.set_buffer_size_near(PERIOD as alsa::pcm::Frames * 10).map_err(failed)?;
        pcm.hw_params(&hw).map_err(failed)?;
    }
    pcm.start().map_err(failed)?;
    Ok(pcm)
}

/// Read `pcm` into `mono` (scaled as cpal's samples are) until `stop` says
/// so or the Recorder is dropped: within WAIT_MS, whatever the source does.
pub fn capture(pcm: &PCM, mono: &Mutex<Vec<f32>>, on_level: &OnLevel, stop: &Receiver<()>) {
    let Ok(io) = pcm.io_i16() else { return };
    let mut buf = [0i16; PERIOD];
    while matches!(stop.try_recv(), Err(TryRecvError::Empty)) {
        let read = pcm.wait(Some(WAIT_MS)).and_then(|ready| if ready { io.readi(&mut buf) } else { Ok(0) });
        match read {
            Ok(0) => {}
            Ok(n) => {
                on_level(level(&buf[..n]));
                mono.lock().unwrap().extend(buf[..n].iter().map(|&s| f32::from(s)));
            }
            // Woken, yet nothing to read.
            Err(e) if std::io::Error::from_raw_os_error(e.errno()).kind() == std::io::ErrorKind::WouldBlock => {}
            // An overrun (the thread was late): carry on.
            Err(e) => {
                if let Err(e) = pcm.try_recover(e, true) {
                    eprintln!("microphone: {e}");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::{output_within, parse};
    use crate::recorder::Microphone;

    #[test]
    fn commands_have_a_deadline() {
        let deadline = Duration::from_millis(200);
        assert_eq!(output_within(Command::new("echo").arg("hi"), deadline), Some(b"hi\n".to_vec()));
        assert_eq!(output_within(Command::new("sh").args(["-c", "echo hi; exit 1"]), deadline), None);
        assert_eq!(output_within(&mut Command::new("localflow-no-such-program"), deadline), None);
        // A PipeWire that doesn't answer: killed at the deadline.
        let start = Instant::now();
        assert_eq!(output_within(Command::new("sleep").arg("5"), deadline), None);
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
        // Printed, then hung.
        assert_eq!(output_within(Command::new("sh").args(["-c", "echo hi; exec sleep 5 >&-"]), deadline), None);
        assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    }

    /// pw-dump's output, cut down to what's read.
    const DUMP: &str = r#"[
      { "id": 0, "type": "PipeWire:Interface:Core", "info": { "props": {} } },
      { "id": 51, "type": "PipeWire:Interface:Node", "info": { "props": {
          "media.class": "Audio/Source", "node.name": "alsa_input.pci-0000_2f_00.4.analog-stereo",
          "node.description": "Starship/Matisse HD Audio Controller Analog Stereo", "node.nick": "ALC897 Analog" } } },
      { "id": 67, "type": "PipeWire:Interface:Node", "info": { "props": {
          "media.class": "Audio/Source", "node.name": "alsa_input.usb-USB_Audio_Device_KREO_KAST-00.analog-stereo",
          "node.description": "KREO KAST Analog Stereo" } } },
      { "id": 70, "type": "PipeWire:Interface:Node", "info": { "props": {
          "media.class": "Audio/Sink", "node.name": "alsa_output.pci-0000_2f_00.4.analog-stereo" } } },
      { "id": 71, "type": "PipeWire:Interface:Node", "info": { "props": {
          "media.class": "Stream/Input/Audio", "node.name": "firefox" } } },
      { "id": 90, "type": "PipeWire:Interface:Node", "info": { "props": {
          "media.class": "Audio/Source", "node.name": "bluez_input.41:42:FF:A6:18:26", "node.nick": "ZEB-THUNDER" } } },
      { "id": 40, "type": "PipeWire:Interface:Metadata", "props": { "metadata.name": "default" }, "metadata": [
          { "subject": 0, "key": "default.audio.sink", "type": "Spa:String:JSON", "value": { "name": "alsa_output.pci-0000_2f_00.4.analog-stereo" } },
          { "subject": 0, "key": "default.audio.source", "type": "Spa:String:JSON", "value": { "name": "alsa_input.usb-USB_Audio_Device_KREO_KAST-00.analog-stereo" } } ] }
    ]"#;

    fn mic(id: &str, name: &str, default: bool) -> Microphone {
        Microphone { id: id.into(), name: name.into(), default }
    }

    #[test]
    fn sources_from_pw_dump() {
        assert_eq!(parse(DUMP).unwrap(), vec![
            mic("alsa_input.pci-0000_2f_00.4.analog-stereo", "Starship/Matisse HD Audio Controller Analog Stereo", false),
            mic("alsa_input.usb-USB_Audio_Device_KREO_KAST-00.analog-stereo", "KREO KAST Analog Stereo", true),
            mic("bluez_input.41:42:FF:A6:18:26", "ZEB-THUNDER", false),
        ]);
        // The default as a JSON string.
        let old = DUMP.replace(
            r#""value": { "name": "alsa_input.usb-USB_Audio_Device_KREO_KAST-00.analog-stereo" }"#,
            r#""value": "{ \"name\": \"bluez_input.41:42:FF:A6:18:26\" }""#,
        );
        let defaults: Vec<String> = parse(&old).unwrap().into_iter().filter(|m| m.default).map(|m| m.id).collect();
        assert_eq!(defaults, ["bluez_input.41:42:FF:A6:18:26"]);
        assert_eq!(parse("[]"), Some(vec![])); // PipeWire with no microphone
        assert_eq!(parse("pw-dump: can't connect"), None);
    }
}
