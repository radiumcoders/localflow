//! Windows: the focused window, the clipboard and paste keys, the keyboard
//! hook, and the focused text box through UI Automation (its COM interfaces,
//! called through their vtables, as the Python version did).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use localflow_core::Part;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, SysFreeString, WPARAM};
use windows_sys::Win32::System::Com::{CLSIDFromString, CoCreateInstance, CoInitializeEx};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW};
use windows_sys::Win32::System::Variant::{VARIANT, VariantClear};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, keybd_event};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW, TranslateMessage,
};
use windows_sys::core::GUID;

use crate::hotkey::HotkeyEvent;
use crate::target::Window;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

// --- The focused window ------------------------------------------------------

pub fn focused_window() -> Window {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return Window::default();
        }
        let mut title = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut app = String::new();
        let process = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if !process.is_null() {
            let (mut buf, mut size) = (vec![0u16; 1024], 1024u32);
            if QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) != 0 {
                let exe = PathBuf::from(String::from_utf16_lossy(&buf[..size as usize]));
                app = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            }
            CloseHandle(process);
        }
        Window { app, title: String::from_utf16_lossy(&title[..n]), pid: Some(pid), tags: vec![] }
    }
}

/// The foreground window's handle ("0" when no window has focus).
pub fn foreground() -> Option<String> {
    Some((unsafe { GetForegroundWindow() } as usize).to_string())
}

// --- Clipboard and keys --------------------------------------------------------

fn open_clipboard() -> bool {
    for _ in 0..20 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

pub fn clipboard_get() -> Option<String> {
    if !open_clipboard() {
        return None;
    }
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        let text = if handle.is_null() {
            None
        } else {
            let p = GlobalLock(handle) as *const u16;
            let text = (!p.is_null()).then(|| {
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
            });
            GlobalUnlock(handle);
            text
        };
        CloseClipboard();
        text
    }
}

fn set_data(format: u32, data: &[u8]) -> bool {
    unsafe {
        let mem = GlobalAlloc(0x0002, data.len()); // GMEM_MOVEABLE
        if mem.is_null() {
            return false;
        }
        let p = GlobalLock(mem) as *mut u8;
        if p.is_null() {
            return false;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(mem);
        if !open_clipboard() {
            return false;
        }
        EmptyClipboard();
        let ok = !SetClipboardData(format, mem).is_null(); // the clipboard owns `mem` now
        CloseClipboard();
        ok
    }
}

/// Text on the clipboard, with Windows' line endings.
pub fn clipboard_set(text: &str) -> bool {
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    set_data(CF_UNICODETEXT, &wide(&text).iter().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>())
}

/// Files on the clipboard, as Explorer's Copy puts them (CF_HDROP).
fn set_files(paths: &[&Path]) -> bool {
    let names: String = paths.iter().map(|p| format!("{}\0", p.display())).collect::<String>() + "\0";
    // DROPFILES: offset of the names, a point, fNC, fWide; then the names in UTF-16.
    let mut data = 20u32.to_le_bytes().to_vec();
    data.extend([0u8; 12]);
    data.extend(1u32.to_le_bytes());
    data.extend(names.encode_utf16().flat_map(|c| c.to_le_bytes()));
    set_data(CF_HDROP, &data)
}

pub fn clipboard_set_file(path: &Path) -> bool {
    set_files(&[path]) && unsafe { IsClipboardFormatAvailable(CF_HDROP) } != 0
}

/// Press a key, with Ctrl held if `ctrl`.
fn press(vk: u8, ctrl: bool) {
    unsafe {
        if ctrl {
            keybd_event(VK_CONTROL as u8, 0, 0, 0);
        }
        keybd_event(vk, 0, 0, 0);
        keybd_event(vk, 0, 2, 0); // KEYEVENTF_KEYUP
        if ctrl {
            keybd_event(VK_CONTROL as u8, 0, 2, 0);
        }
    }
}

fn held(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk)) as u16 & 0x8000 != 0 }
}

fn restore_later(old: Option<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        if let Some(old) = old {
            clipboard_set(&old);
        }
    });
}

pub fn paste(parts: &[Part]) -> bool {
    let old = clipboard_get();
    for (i, part) in parts.iter().enumerate() {
        match part {
            Part::File(path) => set_files(&[Path::new(path)]),
            Part::Text(text) => clipboard_set(text),
        };
        std::thread::sleep(Duration::from_millis(50));
        press(b'V', true);
        if i + 1 < parts.len() {
            std::thread::sleep(Duration::from_millis(300)); // let the app read the clipboard before it changes
        }
    }
    restore_later(old);
    true
}

pub fn selected_text() -> Vec<String> {
    // Wait for the learn key's modifiers to be let go, or the copy is Ctrl+Alt+C.
    for _ in 0..100 {
        if ![VK_SHIFT, VK_CONTROL, VK_MENU, 0x5B, 0x5C].iter().any(|&vk| held(vk)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let old = clipboard_get();
    press(b'C', true);
    std::thread::sleep(Duration::from_millis(250));
    let new = clipboard_get();
    if let Some(old) = &old {
        clipboard_set(old);
    }
    [new, old].map(|t| t.unwrap_or_default().replace("\r\n", "\n")).to_vec()
}

// --- The hotkey ------------------------------------------------------------------

struct Hook {
    tx: Sender<HotkeyEvent>,
    recording: Arc<AtomicBool>,
    test_injected: bool,
}

static HOOK: Mutex<Option<Hook>> = Mutex::new(None);
static HELD: AtomicBool = AtomicBool::new(false);
static LEARN_HELD: AtomicBool = AtomicBool::new(false);

/// Our keys, or None to pass the key on.
fn on_key(kb: &KBDLLHOOKSTRUCT, down: bool, hook: &Hook) -> Option<HotkeyEvent> {
    let injected = kb.flags & 0x10 != 0 && !hook.test_injected; // LLKHF_INJECTED: tests send keys
    if injected {
        return None;
    }
    let send = |e: HotkeyEvent| {
        let _ = hook.tx.send(e);
    };
    if kb.vkCode == VK_SPACE {
        let (ctrl, shift, alt) = (held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU));
        if down && (HELD.load(Ordering::SeqCst) || (ctrl && shift && !alt)) {
            if !HELD.swap(true, Ordering::SeqCst) {
                send(HotkeyEvent::Press);
            }
            return Some(HotkeyEvent::Press);
        }
        if !down && HELD.swap(false, Ordering::SeqCst) {
            send(HotkeyEvent::Release);
            return Some(HotkeyEvent::Release);
        }
        if down && (LEARN_HELD.load(Ordering::SeqCst) || (ctrl && alt && !shift)) {
            if !LEARN_HELD.swap(true, Ordering::SeqCst) {
                send(HotkeyEvent::Learn);
            }
            return Some(HotkeyEvent::Learn);
        }
        if !down && LEARN_HELD.swap(false, Ordering::SeqCst) {
            return Some(HotkeyEvent::Learn);
        }
    }
    if kb.vkCode == VK_ESCAPE && down && hook.recording.load(Ordering::SeqCst) {
        send(HotkeyEvent::Escape);
        return Some(HotkeyEvent::Escape);
    }
    None
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == 0 {
        let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam as u32, 0x100 | 0x104); // WM_KEYDOWN, WM_SYSKEYDOWN
        let ours = HOOK.lock().ok().and_then(|h| h.as_ref().and_then(|h| on_key(kb, down, h)));
        if ours.is_some() {
            return 1; // swallowed
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

pub fn listen(tx: Sender<HotkeyEvent>, recording: Arc<AtomicBool>) -> Result<(), String> {
    let test_injected = std::env::var_os("DICTATE_TEST_INJECTED").is_some();
    *HOOK.lock().unwrap() = Some(Hook { tx: tx.clone(), recording, test_injected });
    unsafe {
        let hook = SetWindowsHookExW(13, Some(hook_proc), GetModuleHandleW(std::ptr::null()), 0); // WH_KEYBOARD_LL
        if hook.is_null() {
            return Err(format!("couldn't install the keyboard hook ({})", std::io::Error::last_os_error()));
        }
        let _ = tx.send(HotkeyEvent::Ready);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut() as HWND, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

// --- The focused text box, through UI Automation ------------------------------------

/// A COM object's method `index`, from its vtable.
unsafe fn method<F: Copy>(obj: *mut c_void, index: usize) -> F {
    unsafe {
        let vtable = *(obj as *const *const usize);
        std::mem::transmute_copy(&*vtable.add(index))
    }
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        unsafe { method::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj) };
    }
}

fn guid(text: &str) -> GUID {
    let mut g = GUID::from_u128(0);
    unsafe { CLSIDFromString(wide(text).as_ptr(), &mut g) };
    g
}

enum Prop {
    Int(i32),
    Text(String),
}

/// IUIAutomation, one per thread (COM objects belong to the thread that made them).
struct Uia(*mut c_void);

impl Uia {
    fn new() -> Result<Uia, String> {
        unsafe {
            CoInitializeEx(std::ptr::null(), 0); // COINIT_MULTITHREADED; already initialized is fine
            let (clsid, iid) = (guid("{ff48dba4-60ef-4201-aa87-54103eef594e}"), guid("{30cbe57d-d9d0-452a-ab13-7ac5ac4825ee}"));
            let mut uia = std::ptr::null_mut();
            let hr = CoCreateInstance(&clsid, std::ptr::null_mut(), 1, &iid, &mut uia);
            if hr < 0 || uia.is_null() {
                return Err(format!("UI Automation unavailable ({:#x})", hr as u32));
            }
            Ok(Uia(uia))
        }
    }

    /// A property of an element: an int or a string.
    unsafe fn prop(element: *mut c_void, id: i32) -> Option<Prop> {
        unsafe {
            let mut var: VARIANT = std::mem::zeroed();
            let get: unsafe extern "system" fn(*mut c_void, i32, *mut VARIANT) -> i32 = method(element, 10);
            if get(element, id, &mut var) < 0 {
                return None;
            }
            let inner = &var.Anonymous.Anonymous;
            let value = match inner.vt {
                3 => Some(Prop::Int(inner.Anonymous.lVal)), // VT_I4
                8 => {
                    // VT_BSTR
                    let p = inner.Anonymous.bstrVal;
                    let len = if p.is_null() { 0 } else { (0..).take_while(|&i| *p.add(i) != 0).count() };
                    Some(Prop::Text(if p.is_null() { String::new() } else { String::from_utf16_lossy(std::slice::from_raw_parts(p, len)) }))
                }
                _ => None,
            };
            VariantClear(&mut var);
            value
        }
    }

    /// All the text of an element with the Text pattern (rich text boxes, browsers).
    unsafe fn document_text(element: *mut c_void) -> Option<String> {
        unsafe {
            let (mut pattern, mut doc, mut bstr) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null::<u16>());
            let iid = guid("{32eba289-3583-42c9-9c59-3b6d9a1e9b6a}"); // IUIAutomationTextPattern
            let as_pattern: unsafe extern "system" fn(*mut c_void, i32, *const GUID, *mut *mut c_void) -> i32 = method(element, 14);
            let mut text = None;
            if as_pattern(element, 10014, &iid, &mut pattern) >= 0 && !pattern.is_null() {
                let range: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = method(pattern, 7); // DocumentRange
                if range(pattern, &mut doc) >= 0 && !doc.is_null() {
                    let get_text: unsafe extern "system" fn(*mut c_void, i32, *mut *const u16) -> i32 = method(doc, 12);
                    if get_text(doc, -1, &mut bstr) >= 0 {
                        let len = if bstr.is_null() { 0 } else { (0..).take_while(|&i| *bstr.add(i) != 0).count() };
                        text = Some(if bstr.is_null() { String::new() } else { String::from_utf16_lossy(std::slice::from_raw_parts(bstr, len)) });
                    }
                }
            }
            if !bstr.is_null() {
                SysFreeString(bstr);
            }
            release(doc);
            release(pattern);
            text
        }
    }

    /// (pid, text) of the focused element, if it holds text.
    fn focused_text(&self) -> Option<(u32, String)> {
        unsafe {
            let mut element = std::ptr::null_mut();
            let focused: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = method(self.0, 8); // GetFocusedElement
            if focused(self.0, &mut element) < 0 || element.is_null() {
                return None;
            }
            let pid = match Self::prop(element, 30002) {
                Some(Prop::Int(pid)) => pid as u32, // UIA_ProcessIdPropertyId
                _ => 0,
            };
            let text = match Self::prop(element, 30045) {
                Some(Prop::Text(text)) if !text.is_empty() => Some(text), // UIA_ValueValuePropertyId
                _ => Self::document_text(element),
            };
            release(element);
            text.filter(|t| !t.is_empty()).map(|t| (pid, t))
        }
    }
}

thread_local! {
    static UIA: Result<Uia, String> = Uia::new();
}

pub fn focused_text() -> Option<(u32, String)> {
    UIA.with(|uia| match uia {
        Ok(uia) => uia.focused_text(),
        Err(e) => {
            crate::paths::info(format_args!("can't read the text box: {e}"));
            None
        }
    })
}
