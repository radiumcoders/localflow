//! The clipboard round trip the self-test does: text with unicode and
//! newlines, then a file; your clipboard is put back after.

use localflow_desktop::paste::{clipboard_get, clipboard_set, clipboard_set_file};

fn main() {
    let old = clipboard_get();
    let sample = "localflow ✓ test\nline two";
    let set = clipboard_set(sample);
    let got = clipboard_get();
    let file = clipboard_set_file(std::path::Path::new(file!()));
    if let Some(old) = old {
        clipboard_set(&old);
    }
    println!("set {set}, read back the same: {}, file: {file}", got.as_deref() == Some(sample));
}
