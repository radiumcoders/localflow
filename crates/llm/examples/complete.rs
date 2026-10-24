//! Complete a prompt with a GGUF model, timing load and answer:
//!
//!     cargo run --release -p localflow-llm --example complete -- MODEL.gguf "PROMPT" [--cpu]

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(path), Some(prompt)) = (args.first(), args.get(1)) else {
        eprintln!("usage: complete MODEL.gguf PROMPT [--cpu]");
        std::process::exit(2);
    };
    let gpu = !args.iter().any(|a| a == "--cpu");
    let t = Instant::now();
    let mut model = localflow_llm::Model::load(path.as_ref(), gpu, 4096).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("loaded in {} ms", t.elapsed().as_millis());
    for _ in 0..2 {
        let t = Instant::now();
        let (text, cut_off) = model.complete(prompt, 512).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("answered in {} ms{}", t.elapsed().as_millis(), if cut_off { " (cut off)" } else { "" });
        println!("{text}");
    }
}
