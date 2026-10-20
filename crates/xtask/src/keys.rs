//! Injected key presses for the end-to-end test: the dictation hotkey, which
//! the app only takes from a program when DICTATE_TEST_INJECTED is set.

/// Press (down) or let go of the hotkey: Ctrl+Shift+Space on Windows,
/// Option+Shift+Space on macOS.
pub fn hotkey(down: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, keybd_event};
        let keys: [(u8, u32); 3] =
            if down { [(0x11, 0), (0x10, 0), (0x20, 0)] } else { [(0x20, KEYEVENTF_KEYUP), (0x10, KEYEVENTF_KEYUP), (0x11, KEYEVENTF_KEYUP)] };
        for (vk, flags) in keys {
            unsafe { keybd_event(vk, 0, flags, 0) };
        }
    }
    #[cfg(target_os = "macos")]
    {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
        if let Ok(src) = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            && let Ok(e) = CGEvent::new_keyboard_event(src, 49, down)
        {
            e.set_flags(CGEventFlags::CGEventFlagAlternate | CGEventFlags::CGEventFlagShift);
            e.post(CGEventTapLocation::HID);
        }
    }
    let _ = down;
}
