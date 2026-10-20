//! file_candidates for each line of stdin, over a project's files: which
//! paths a dictation would offer the cleanup.
//!
//!     cargo run -p localflow-desktop --example candidates -- PROJECT < sentences.txt

use std::io::BufRead;

use localflow_desktop::project::{file_candidates, project_files};

fn main() {
    let root = std::env::args().nth(1).expect("a project folder");
    let files = project_files(root.as_ref());
    for line in std::io::stdin().lock().lines() {
        println!("{}", serde_json::to_string(&file_candidates(&line.unwrap(), &files, 12)).unwrap());
    }
}
