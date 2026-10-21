//! macOS: the frontmost app and its window title, the clipboard and paste
//! keys, the keyboard tap for the hotkey, and the focused text box through
//! the Accessibility API. The keys and the window title need the
//! Accessibility permission (System Settings > Privacy & Security).

use std::ffi::c_void;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use localflow_core::Part;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

use crate::hotkey::HotkeyEvent;
use crate::target::Window;

type AXUIElementRef = *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: CFTypeRef) -> i32;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
}

// AppKit holds NSWorkspace and NSPasteboard; looked up by name at run time.
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
}

const CMD: u64 = 0x100000;
const SHIFT: u64 = 0x20000;
const ALT: u64 = 0x80000;
const CTRL: u64 = 0x40000;

// --- The frontmost app ------------------------------------------------------------

fn frontmost_app() -> *mut AnyObject {
    unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        msg_send![ws, frontmostApplication]
    }
}

pub fn frontmost_pid() -> Option<i32> {
    let app = frontmost_app();
    (!app.is_null()).then(|| unsafe { msg_send![app, processIdentifier] })
}

/// An NSString / CFString as a Rust string.
unsafe fn ns_text(obj: *const c_void) -> String {
    if obj.is_null() {
        return String::new();
    }
    unsafe { CFString::wrap_under_get_rule(obj as CFStringRef) }.to_string()
}

/// An attribute of an accessibility element (+1: release it), or null.
unsafe fn ax_copy(element: AXUIElementRef, name: &str) -> CFTypeRef {
    let mut value: CFTypeRef = std::ptr::null();
    let attr = CFString::new(name);
    let err = unsafe { AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) };
    if err == 0 { value } else { std::ptr::null() }
}

fn is_string(value: CFTypeRef) -> bool {
    unsafe { CFType::wrap_under_get_rule(value) }.instance_of::<CFString>()
}

/// (bundle id, window title, pid). The title needs the Accessibility
/// permission; without it, it's empty.
pub fn focused_window() -> Window {
    let app = frontmost_app();
    if app.is_null() {
        return Window::default();
    }
    unsafe {
        let pid: i32 = msg_send![app, processIdentifier];
        let bundle: *mut AnyObject = msg_send![app, bundleIdentifier];
        let mut title = String::new();
        let element = AXUIElementCreateApplication(pid);
        if !element.is_null() {
            let window = ax_copy(element, "AXFocusedWindow");
            if !window.is_null() {
                let value = ax_copy(window as AXUIElementRef, "AXTitle");
                if !value.is_null() {
                    if is_string(value) {
                        title = ns_text(value);
                    }
                    CFRelease(value);
                }
                CFRelease(window);
            }
            CFRelease(element);
        }
        Window { app: ns_text(bundle as *const c_void), title, pid: Some(pid as u32), tags: vec![] }
    }
}

static MANUAL: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// (pid, text) of the focused text box.
pub fn focused_text() -> Option<(u32, String)> {
    unsafe {
        let system = AXUIElementCreateSystemWide();
        let element = ax_copy(system, "AXFocusedUIElement");
        let mut found = None;
        if !element.is_null() {
            let mut pid = 0;
            AXUIElementGetPid(element as AXUIElementRef, &mut pid);
            let mut asked = MANUAL.lock().unwrap();
            if !asked.contains(&pid) {
                // Electron apps (Cursor, Slack...) only fill in their tree when asked.
                asked.push(pid);
                let app = AXUIElementCreateApplication(pid);
                if !app.is_null() {
                    let attr = CFString::new("AXManualAccessibility");
                    AXUIElementSetAttributeValue(app, attr.as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef());
                    CFRelease(app);
                }
            }
            let value = ax_copy(element as AXUIElementRef, "AXValue");
            if !value.is_null() {
                if is_string(value) {
                    found = Some((pid as u32, ns_text(value)));
                }
                CFRelease(value);
            }
            CFRelease(element);
        }
        if !system.is_null() {
            CFRelease(system);
        }
        found
    }
}

// --- Keys and the clipboard ------------------------------------------------------------

pub(crate) fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// macOS's own prompt to allow this app under Accessibility; it also lists the
/// app there, ready to switch on.
pub fn ask_accessibility() {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) };
}

/// Press a key with modifier flags.
fn key(code: u16, flags: u64) {
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else { return };
    for down in [true, false] {
        if let Ok(e) = CGEvent::new_keyboard_event(source.clone(), code, down) {
            e.set_flags(CGEventFlags::from_bits_truncate(flags));
            e.post(CGEventTapLocation::HID);
        }
    }
}

pub fn press_key(k: crate::paste::Key) -> bool {
    if !trusted() {
        return false;
    }
    key(match k {
        crate::paste::Key::Enter => 36,
        crate::paste::Key::Escape => 53,
    }, 0);
    true
}

fn modifiers_held() -> bool {
    unsafe { CGEventSourceFlagsState(1) & (CMD | SHIFT | ALT | CTRL) != 0 }
}

fn pb(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("LC_CTYPE", "UTF-8");
    cmd
}

fn pbpaste() -> Vec<u8> {
    pb("pbpaste").stdin(Stdio::null()).stderr(Stdio::null()).output().map(|o| o.stdout).unwrap_or_default()
}

fn pbcopy(data: &[u8]) -> bool {
    let Ok(mut child) = pb("pbcopy").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let wrote = child.stdin.take().is_some_and(|mut s| s.write_all(data).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}

pub fn clipboard_get() -> Option<String> {
    pb("pbpaste").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn clipboard_set(text: &str) -> bool {
    pbcopy(text.as_bytes())
}

/// A file on the clipboard, as Finder's Copy puts it.
fn copy_file(path: &Path) -> bool {
    let path = CFString::new(&path.to_string_lossy());
    unsafe {
        let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: path.as_concrete_TypeRef() as *mut AnyObject];
        let items: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: url];
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: isize = msg_send![board, clearContents];
        msg_send![board, writeObjects: items]
    }
}

pub fn clipboard_set_file(path: &Path) -> bool {
    copy_file(path) && unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let kind = CFString::new("public.file-url");
        let types: *mut AnyObject = msg_send![board, types];
        let has: bool = msg_send![types, containsObject: kind.as_concrete_TypeRef() as *mut AnyObject];
        has
    }
}

pub fn paste(parts: &[Part]) -> bool {
    let old = pbpaste();
    if !trusted() {
        // Key presses would be dropped silently: leave the text on the clipboard.
        let text: String = parts.iter().map(|p| match p { Part::Text(t) | Part::File(t) => t.as_str() }).collect();
        pbcopy(text.as_bytes());
        return false;
    }
    for (i, part) in parts.iter().enumerate() {
        match part {
            Part::File(path) => copy_file(Path::new(path)),
            Part::Text(text) => pbcopy(text.as_bytes()),
        };
        std::thread::sleep(Duration::from_millis(50));
        key(9, CMD); // v
        if i + 1 < parts.len() {
            std::thread::sleep(Duration::from_millis(300)); // let the app read the clipboard before it changes
        }
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        pbcopy(&old);
    });
    true
}

pub fn selected_text() -> Vec<String> {
    // Wait for the learn key's modifiers to be let go, or the copy is Option+Shift+Cmd+C.
    for _ in 0..100 {
        if !modifiers_held() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let old = pbpaste();
    key(8, CMD); // c
    std::thread::sleep(Duration::from_millis(250));
    let new = pbpaste();
    pbcopy(&old);
    [new, old].map(|t| String::from_utf8_lossy(&t).into_owned()).to_vec()
}

// --- The hotkey --------------------------------------------------------------------------

static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
const SPACE: i64 = 49;
const ESC: i64 = 53;
const L: i64 = 37;

pub fn listen(tx: Sender<HotkeyEvent>, recording: Arc<AtomicBool>) -> Result<(), String> {
    let (held, learn_held) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let make_tap = || {
        let (tx, recording, held, learn_held) = (tx.clone(), recording.clone(), held.clone(), learn_held.clone());
        CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default, // can swallow
            vec![CGEventType::KeyDown, CGEventType::KeyUp],
            move |_proxy, kind, event| {
                let send = |e| {
                    let _ = tx.send(e);
                };
                let (down, up) = (matches!(kind, CGEventType::KeyDown), matches!(kind, CGEventType::KeyUp));
                if !down && !up {
                    // Disabled by a timeout or user input: switch it back on.
                    let tap = TAP.load(Ordering::SeqCst);
                    if !tap.is_null() {
                        unsafe { CGEventTapEnable(tap, true) };
                    }
                    return CallbackResult::Keep;
                }
                let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                let flags = event.get_flags().bits();
                let ours = flags & ALT != 0 && flags & SHIFT != 0 && flags & (CMD | CTRL) == 0;
                if code == SPACE {
                    if down && (held.load(Ordering::SeqCst) || ours) {
                        if !held.swap(true, Ordering::SeqCst) {
                            send(HotkeyEvent::Press);
                        }
                        return CallbackResult::Drop;
                    }
                    if up && held.swap(false, Ordering::SeqCst) {
                        send(HotkeyEvent::Release);
                        return CallbackResult::Drop;
                    }
                }
                if code == L {
                    // Option+Shift+L: learn
                    if down && (learn_held.load(Ordering::SeqCst) || ours) {
                        if !learn_held.swap(true, Ordering::SeqCst) {
                            send(HotkeyEvent::Learn);
                        }
                        return CallbackResult::Drop;
                    }
                    if up && learn_held.swap(false, Ordering::SeqCst) {
                        return CallbackResult::Drop;
                    }
                }
                if code == ESC && down && recording.load(Ordering::SeqCst) {
                    send(HotkeyEvent::Escape);
                    return CallbackResult::Drop;
                }
                CallbackResult::Keep
            },
        )
    };
    let tap = match make_tap() {
        Ok(tap) => tap,
        Err(()) => {
            let msg = "no keyboard access: allow localflow in System Settings > Privacy & Security > Accessibility";
            crate::paths::error(format_args!("hotkey: {msg}"));
            let _ = tx.send(HotkeyEvent::Error(msg.into()));
            ask_accessibility();
            loop {
                // Allowing it takes effect without a restart.
                std::thread::sleep(Duration::from_secs(2));
                if let Ok(tap) = make_tap() {
                    crate::paths::info("keyboard access allowed");
                    break tap;
                }
            }
        }
    };
    TAP.store(tap.mach_port().as_concrete_TypeRef() as *mut c_void, Ordering::SeqCst);
    let source = tap.mach_port().create_runloop_source(0).map_err(|()| "couldn't listen to the keyboard tap".to_string())?;
    unsafe { CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes) };
    tap.enable();
    let _ = tx.send(HotkeyEvent::Ready);
    CFRunLoop::run_current();
    Ok(())
}
