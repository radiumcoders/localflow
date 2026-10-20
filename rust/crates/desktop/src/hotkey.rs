//! The dictation hotkey on Windows and macOS (Linux compositors own their keys,
//! so there key bindings run `localflow start` / `stop`): hold it and talk,
//! or tap it, talk, and tap again. Escape cancels. The hotkey's own key
//! presses are swallowed so the space doesn't reach the app you're typing in.
//!
//! ```text
//! Ctrl+Shift+Space (Windows), Option+Shift+Space (macOS): dictate
//! Ctrl+Alt+Space (Windows), Option+Shift+L (macOS): learn a correction
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    Press,
    Release,
    /// Escape while recording.
    Escape,
    Learn,
    /// The keyboard hook is in place.
    Ready,
    /// The hook can't be installed (yet: on macOS it keeps trying until
    /// Accessibility is allowed, then sends Ready).
    Error(String),
}

/// Install the keyboard hook and run its loop, sending what happens to `tx`.
/// Blocks: run it on a thread of its own. `recording` says whether Escape
/// is ours to take.
pub fn listen(tx: Sender<HotkeyEvent>, recording: Arc<AtomicBool>) -> Result<(), String> {
    #[cfg(windows)]
    return crate::windows::listen(tx, recording);
    #[cfg(target_os = "macos")]
    return crate::macos::listen(tx, recording);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (tx, recording);
        Err("Linux uses compositor key bindings".into())
    }
}

/// The hotkey's names, for messages: (dictate, learn).
pub fn names() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") { ("Option+Shift+Space", "Option+Shift+L") } else { ("Ctrl+Shift+Space", "Ctrl+Alt+Space") }
}

/// A dictation the hotkey started.
pub trait Dictation: Send + 'static {
    /// Stop recording and throw it away.
    fn cancel(self);
}

/// A press shorter than this (seconds) is a tap.
pub const TAP: f64 = 0.35;

type Finish<D> = Arc<dyn Fn(D) + Send + Sync>;
type Message = Arc<dyn Fn(String) + Send + Sync>;
type Learn = Arc<dyn Fn() -> String + Send + Sync>;

/// What the hotkey does: hold to talk (stop on release), or tap to start and
/// tap again to stop; Escape cancels. Finishing and learning run on threads
/// of their own; until they're done, presses are ignored.
pub struct HotkeyController<D: Dictation> {
    /// Makes a dictation and starts recording; Err says why it can't.
    pub new_dictation: Box<dyn FnMut() -> Result<D, String> + Send>,
    finish: Finish<D>,
    message: Message,
    learn: Option<Learn>,
    current: Option<D>,
    pressed: f64,
    toggle: bool,
    working: Arc<AtomicBool>,
    recording: Arc<AtomicBool>,
}

impl<D: Dictation> HotkeyController<D> {
    /// `finish` finishes a dictation (blocking; it runs on a thread),
    /// `message` shows a problem.
    pub fn new(
        new_dictation: impl FnMut() -> Result<D, String> + Send + 'static,
        finish: impl Fn(D) + Send + Sync + 'static,
        message: impl Fn(String) + Send + Sync + 'static,
    ) -> Self {
        HotkeyController {
            new_dictation: Box::new(new_dictation),
            finish: Arc::new(finish),
            message: Arc::new(message),
            learn: None,
            current: None,
            pressed: 0.0,
            toggle: false,
            working: Arc::default(),
            recording: Arc::default(),
        }
    }

    /// The learn key: `learn` does the work and returns a message to show.
    pub fn with_learn(mut self, learn: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.learn = Some(Arc::new(learn));
        self
    }

    pub fn recording(&self) -> bool {
        self.current.is_some()
    }

    /// Shared with `listen`, which takes Escape only while recording.
    pub fn recording_flag(&self) -> Arc<AtomicBool> {
        self.recording.clone()
    }

    /// Still finishing a dictation or learning.
    pub fn working(&self) -> &AtomicBool {
        &self.working
    }

    /// Handle a hotkey event at time `now` (seconds, any monotonic clock).
    pub fn on(&mut self, event: &HotkeyEvent, now: f64) {
        match event {
            HotkeyEvent::Press if self.current.is_none() => {
                if !self.working.load(Ordering::SeqCst) {
                    match (self.new_dictation)() {
                        Ok(d) => self.current = Some(d),
                        Err(e) => (self.message)(e.chars().take(40).collect()),
                    }
                    (self.pressed, self.toggle) = (now, false);
                }
            }
            HotkeyEvent::Press if self.toggle => self.finish_current(),
            HotkeyEvent::Release if self.current.is_some() => {
                if now - self.pressed < TAP && !self.toggle {
                    self.toggle = true; // a tap: keep recording until the next tap
                } else if !self.toggle {
                    self.finish_current();
                }
            }
            HotkeyEvent::Escape => {
                if let Some(d) = self.current.take() {
                    d.cancel();
                }
            }
            HotkeyEvent::Learn if self.current.is_none() && !self.working.load(Ordering::SeqCst) => {
                let Some(learn) = self.learn.clone() else { return };
                self.working.store(true, Ordering::SeqCst);
                let (working, message) = (self.working.clone(), self.message.clone());
                std::thread::spawn(move || {
                    let _done = Done(working);
                    message(learn());
                });
            }
            _ => {}
        }
        self.recording.store(self.current.is_some(), Ordering::SeqCst);
    }

    fn finish_current(&mut self) {
        let Some(d) = self.current.take() else { return };
        self.working.store(true, Ordering::SeqCst);
        let (working, finish) = (self.working.clone(), self.finish.clone());
        std::thread::spawn(move || {
            let _done = Done(working);
            finish(d);
        });
    }
}

/// Clears the working flag when the work ends, even if it panics.
struct Done(Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;

    type Log = Arc<Mutex<Vec<(&'static str, usize)>>>;

    struct Fake(usize, Log);

    impl Dictation for Fake {
        fn cancel(self) {
            self.1.lock().unwrap().push(("cancel", self.0));
        }
    }

    fn wait_idle(keys: &HotkeyController<Fake>) {
        for _ in 0..200 {
            if !keys.working().load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("still working");
    }

    fn kinds(log: &Log) -> Vec<&'static str> {
        log.lock().unwrap().iter().map(|e| e.0).collect()
    }

    #[test]
    fn hold_tap_escape() {
        let log: Log = Arc::default();
        let (l1, l2, l3) = (log.clone(), log.clone(), log.clone());
        let mut keys = HotkeyController::new(
            move || {
                let n = l1.lock().unwrap().len();
                l1.lock().unwrap().push(("start", n));
                Ok(Fake(n, l1.clone()))
            },
            move |d: Fake| l2.lock().unwrap().push(("finish", d.0)),
            move |_| l3.lock().unwrap().push(("message", 0)),
        );
        // Hold: press, release after 1 s -> finish.
        keys.on(&HotkeyEvent::Press, 0.0);
        assert!(keys.recording() && keys.recording_flag().load(Ordering::SeqCst));
        keys.on(&HotkeyEvent::Release, 1.0);
        assert!(!keys.recording() && !keys.recording_flag().load(Ordering::SeqCst));
        wait_idle(&keys);
        assert_eq!(kinds(&log), ["start", "finish"]);
        // Tap: press + quick release keeps recording; the next press finishes.
        keys.on(&HotkeyEvent::Press, 10.0);
        keys.on(&HotkeyEvent::Release, 10.1);
        assert!(keys.recording());
        keys.on(&HotkeyEvent::Press, 15.0);
        wait_idle(&keys);
        keys.on(&HotkeyEvent::Release, 15.1); // release of the stopping tap: nothing happens
        assert_eq!(kinds(&log), ["start", "finish", "start", "finish"]);
        // Escape cancels.
        keys.on(&HotkeyEvent::Press, 20.0);
        keys.on(&HotkeyEvent::Escape, 20.5);
        assert!(!keys.recording());
        assert_eq!(kinds(&log).last(), Some(&"cancel"));
        keys.on(&HotkeyEvent::Release, 21.0);
        assert_eq!(kinds(&log).last(), Some(&"cancel"));
    }

    #[test]
    fn busy_and_errors() {
        let messages: Arc<Mutex<Vec<String>>> = Arc::default();
        let shown = messages.clone();
        let mut keys = HotkeyController::new(
            || Err::<Fake, _>("no microphone, and a long explanation of why not".into()),
            |_| {},
            move |m| shown.lock().unwrap().push(m),
        );
        keys.on(&HotkeyEvent::Press, 0.0);
        assert!(!keys.recording());
        assert_eq!(*messages.lock().unwrap(), ["no microphone, and a long explanation of"]);
        keys.working().store(true, Ordering::SeqCst); // still finishing the last one: presses are ignored
        keys.new_dictation = Box::new(|| panic!("started while busy"));
        keys.on(&HotkeyEvent::Press, 1.0);
        assert!(!keys.recording());
    }

    #[test]
    fn learn_key() {
        let shown: Arc<Mutex<Vec<String>>> = Arc::default();
        let s = shown.clone();
        let mut keys = HotkeyController::new(|| -> Result<Fake, String> { panic!("no dictation") }, |_| {}, move |m| s.lock().unwrap().push(m))
            .with_learn(|| "Learned: Claw → Claude".into());
        keys.on(&HotkeyEvent::Learn, 0.0);
        wait_idle(&keys);
        assert_eq!(*shown.lock().unwrap(), ["Learned: Claw → Claude"]);
    }
}
