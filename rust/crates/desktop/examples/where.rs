//! Where would a dictation go right now? Prints the focused window, its
//! project, and the files a sentence would offer the cleanup.
//!
//!     cargo run -p localflow-desktop --example where -- "look at the install file"

use std::time::Instant;

use localflow_desktop::project::file_candidates;
use localflow_desktop::target::{Target, focused_window, foreground};

fn main() {
    let sentence = std::env::args().nth(1).unwrap_or_else(|| "refer to the read me file".into());
    let start = Instant::now();
    let window = focused_window();
    println!("window: {window:?} (foreground {:?})", foreground());
    let target = Target::new(&window);
    println!("target: {target} in {} ms (cwd {:?})", start.elapsed().as_millis(), target.cwd);
    let files = file_candidates(&sentence, &target.files, 12);
    println!("{sentence:?} offers: {files:?}");
    if let Some(first) = files.first() {
        println!("referenced as: {:?}", target.reference(first));
    }
}
