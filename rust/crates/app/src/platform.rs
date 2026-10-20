//! The few OS calls the app makes itself (the rest is in localflow-desktop).

use gpui_kit::Window;

/// The overlay is never clicked and never activated: on Windows it must not
/// take focus when shown, on macOS clicks go through it.
pub fn overlay_window(window: &mut Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        };
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::Win32(h) = handle.as_raw()
        {
            let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let extra = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST) as isize;
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | extra);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = window.window_handle()
            && let RawWindowHandle::AppKit(h) = handle.as_raw()
        {
            unsafe {
                let view = h.ns_view.as_ptr() as *mut AnyObject;
                let ns_window: *mut AnyObject = msg_send![view, window];
                if !ns_window.is_null() {
                    let _: () = msg_send![ns_window, setIgnoresMouseEvents: true];
                }
            }
        }
    }
    let _ = window;
}

/// macOS: no Dock icon or menu bar; localflow is a helper you talk to, not an
/// app you switch to. The settings window still opens in front.
pub fn background_app() {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
    }
}

/// Windows: the app has no console of its own (it runs in the background),
/// so commands run from a terminal print to that terminal.
pub fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
