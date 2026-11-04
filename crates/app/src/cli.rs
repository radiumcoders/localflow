//! Commands for a terminal: speech and cleanup models, recording and transcribing by
//! hand, the cleanup as a filter, and its checks.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use localflow_core::local::MODEL;
use localflow_core::diagram::{self, Request};
use localflow_core::{Config, Localflow, NO_FILES, eval};
use localflow_desktop::history;
use localflow_desktop::paths::utc;
use localflow_speech::{RATE, Recorder, level, microphones, models, read_wav, wav_bytes};

const USAGE: &str = "localflow: talk, and get clean text where you type

    localflow                        # settings (Linux), or the background app (macOS / Windows)
    localflow settings
    localflow onboarding             # the first-run window, again
    localflow quit                   # stop the background app (macOS / Windows)
    localflow setup [--model FILE.gguf] [--speech-model ID]
    localflow start [raw] | stop | toggle [raw] | cancel | learn
    localflow history [N]            # the last N dictations (default 10): what was heard, what went out
    localflow history --last         # just the last text that went out
    localflow history --clear
    localflow snippets               # your snippets: what each trigger pastes
    localflow visualize TEXT|-       # draw a diagram of TEXT (\"a flowchart of ...\"): Mermaid on
                                     # stdout, the PNG's path on stderr
    localflow models [download ID]   # speech and cleanup models: list, or download one
    localflow mics                   # the microphones, with the ids LOCALFLOW_MIC takes
    localflow listen [SECONDS]       # record (default 5 s), transcribe, clean up, print
    localflow transcribe FILE.wav    # a 16-bit WAV through the speech model and the cleanup
    localflow record SECONDS FILE.wav
    echo \"meet at 3 wait at 2\" | localflow -
    localflow --check                # does the cleanup model load?
    localflow --eval                 # run cases.txt through the cleanup model
    localflow --selftest             # check each piece that doesn't need a person
    localflow --version";

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("localflow: {e}");
    ExitCode::FAILURE
}

pub fn run(args: &[String]) -> ExitCode {
    let lf = Localflow::new(Config::from_env());
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0).unwrap_or_default() {
        "--check" => match lf.warm() {
            Ok(()) => println!("ok: {}", lf.cfg.model),
            Err(e) => return fail(format!("{}: {e}", lf.cfg.model)),
        },
        "--eval" => {
            if let Err(e) = eval::run(&lf) {
                return fail(e);
            }
        }
        "models" => return models_command(&lf.cfg, &args[1..]),
        "mics" => return mics_command(&lf.cfg),
        "history" => return history_command(&lf.cfg, &args[1..]),
        "snippets" => snippets_command(&lf),
        "visualize" | "visualise" => return visualize_command(&lf, &args[1..]),
        "listen" => {
            let seconds = arg(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
            return match record(lf.cfg.mic.as_deref(), seconds) {
                Ok(samples) => dictate(&lf, &samples),
                Err(e) => fail(e),
            };
        }
        "transcribe" => {
            let Some(path) = arg(1) else { return usage() };
            return match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| read_wav(&b).map_err(|e| e.0)) {
                Ok(samples) => dictate(&lf, &samples),
                Err(e) => fail(format!("{path}: {e}")),
            };
        }
        "record" => {
            let (Some(seconds), Some(path)) = (arg(1).and_then(|s| s.parse().ok()), arg(2)) else { return usage() };
            match record(lf.cfg.mic.as_deref(), seconds).and_then(|s| std::fs::write(path, wav_bytes(&s, RATE)).map(|_| s).map_err(|e| e.to_string())) {
                Ok(samples) => eprintln!("  {path}: {:.1}s, level {:.2}", samples.len() as f32 / RATE as f32, level(&samples)),
                Err(e) => return fail(e),
            }
        }
        "-" => {
            let mut text = String::new();
            if std::io::stdin().read_to_string(&mut text).is_err() {
                return fail("stdin isn't text");
            }
            let out = lf.clean(&text, NO_FILES).unwrap_or_else(|e| {
                // No model: pass the text through
                eprintln!("localflow: {e}; passing text through unchanged");
                text.trim().to_string()
            });
            print!("{out}");
            let _ = std::io::stdout().flush();
        }
        "-h" | "--help" | "help" => println!("{USAGE}"),
        "--version" | "-V" => println!("localflow {}", version()),
        other => {
            eprintln!("localflow: unknown command {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

/// The release's version (set by `cargo xtask package`), or the crate's.
pub fn version() -> &'static str {
    option_env!("LOCALFLOW_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn models_command(cfg: &Config, args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("list") => {
            let current = models::resolve(cfg.speech_model.as_deref()).id;
            for m in models::MODELS {
                let mark = if models::installed(m.id) { "✓" } else { " " };
                let chosen = if m.id == current { "  (in use)" } else { "" };
                println!("{mark} {:<24} {:>5} MB  {}{chosen}\n    {}", m.id, m.size_mb(), m.name, m.note);
            }
            let mark = if cfg.local_model_installed() { "✓" } else { " " };
            let chosen = if cfg.model == MODEL.id { "  (in use)" } else { "" };
            println!("\ncleanup:\n{mark} {:<24} {:>5} MB  {}{chosen}", MODEL.id, MODEL.size_mb(), MODEL.name);
            println!("\ndiagrams (\"visualize ...\"):");
            for m in &diagram::MODELS {
                let here = Config { diagram_model: m.id.into(), ..cfg.clone() }.diagram_model_installed();
                let mark = if here { "✓" } else { " " };
                let chosen = if cfg.diagram_model == m.id { "  (in use)" } else { "" };
                println!("{mark} {:<24} {:>5} MB  {}{chosen}", m.id, m.size_mb(), m.name);
            }
            let key = if cfg.anthropic_key.is_some() { "✓" } else { " " };
            let chosen = if cfg.haiku_diagrams() { "  (in use)" } else { "" };
            println!("{key} {:<24} {:>8}  Claude Haiku 5.5, your Anthropic API key (beta){chosen}", diagram::HAIKU, "API");
            if !cfg.diagrams() {
                println!("  (off: LOCALFLOW_DIAGRAM_MODEL=off)");
            }
            println!("\nin {}", models::models_dir().display());
        }
        Some("download") if args.get(1).map(String::as_str) == Some(MODEL.id) => {
            let mut last = u64::MAX;
            let mut progress = |done: u64, total: u64| {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                if pct != last {
                    last = pct;
                    eprint!("\r  {}: {pct}% of {} MB", MODEL.id, total / 1_000_000);
                }
            };
            let cfg = Config { model: MODEL.id.into(), ..cfg.clone() };
            match crate::cleanup_model::get_ready(&cfg, &mut progress, &AtomicBool::new(false)) {
                Ok(()) => eprintln!("\n  {}", cfg.model_file().display()),
                Err(e) => return fail(e),
            }
        }
        Some("download") if diagram::MODELS.iter().any(|m| Some(m.id) == args.get(1).map(String::as_str)) => {
            let cfg = Config { diagram_model: args[1].clone(), ..cfg.clone() };
            let mut last = u64::MAX;
            let mut progress = |done: u64, total: u64| {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                if pct != last {
                    last = pct;
                    eprint!("\r  {}: {pct}% of {} MB", cfg.diagram_model, total / 1_000_000);
                }
            };
            match crate::cleanup_model::get_diagram_ready(&cfg, &mut progress, &AtomicBool::new(false)) {
                Ok(()) => eprintln!("\n  {}", cfg.diagram_model_file().display()),
                Err(e) => return fail(e),
            }
        }
        Some("download") => {
            let Some(id) = args.get(1) else { return usage() };
            let Some(model) = models::MODELS.iter().find(|m| m.id == id) else {
                return fail(format!("no model {id} (see `localflow models`)"));
            };
            let mut last = u64::MAX;
            let mut progress = |done: u64, total: u64| {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                if pct != last {
                    last = pct;
                    eprint!("\r  {}: {pct}% of {} MB", model.id, total / 1_000_000);
                }
            };
            match models::download(model.id, &mut progress, &AtomicBool::new(false)) {
                Ok(path) => eprintln!("\n  {}", path.display()),
                Err(e) => return fail(e),
            }
        }
        _ => return usage(),
    }
    ExitCode::SUCCESS
}

/// The microphones, marking the system's default and the one a dictation
/// records: LOCALFLOW_MIC's when it's connected, else the default. The id
/// LOCALFLOW_MIC takes goes under the name when it's another.
fn mics_command(cfg: &Config) -> ExitCode {
    let mics = microphones();
    if mics.is_empty() {
        return fail("no microphones");
    }
    let chosen = cfg.mic.as_deref().filter(|m| mics.iter().any(|mic| mic.id.trim() == *m));
    for mic in &mics {
        let in_use = chosen.map_or(mic.default, |m| mic.id.trim() == m);
        let tags = match (mic.default, in_use) {
            (true, true) => "  (system default, in use)",
            (true, false) => "  (system default)",
            (false, true) => "  (in use)",
            (false, false) => "",
        };
        println!("  {}{tags}", mic.name);
        if mic.id != mic.name {
            println!("    {}", mic.id);
        }
    }
    if let Some(mic) = cfg.mic.as_deref().filter(|_| chosen.is_none()) {
        println!("\nLOCALFLOW_MIC is {mic}, which isn't available: the system default is recorded");
    }
    ExitCode::SUCCESS
}

fn history_command(cfg: &Config, args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--clear") => {
            if let Err(e) = history::clear() {
                return fail(e);
            }
            println!("history cleared");
        }
        Some("--last") => match history::recent(usize::MAX).into_iter().rev().find(|e| e.outcome != "command") {
            Some(e) => println!("{}", e.text),
            None => return fail("no dictations yet"),
        },
        n => {
            let Ok(n) = n.map_or(Ok(10), str::parse::<usize>) else { return usage() };
            let entries = history::recent(n);
            if entries.is_empty() {
                println!("no dictations yet{}", if cfg.history { "" } else { " (LOCALFLOW_HISTORY is off)" });
            }
            for e in entries {
                let into = match (&e.agent, e.app.is_empty()) {
                    (Some(agent), _) => format!(" into {agent} in {}", e.app),
                    (None, false) => format!(" into {}", e.app),
                    (None, true) => String::new(),
                };
                println!("{}{into} ({})", utc(e.time as i64), e.outcome);
                println!("  heard: {}", e.heard);
                println!("  wrote: {}\n", e.text.replace('\n', "\n         "));
            }
        }
    }
    ExitCode::SUCCESS
}

/// Draw what the arguments (or stdin, for "-") describe, as a dictation
/// would: "a flowchart of …" picks the kind, anything else lets the model.
fn visualize_command(lf: &Localflow, args: &[String]) -> ExitCode {
    let mut said = args.join(" ");
    if said.trim() == "-" {
        said.clear();
        if std::io::stdin().read_to_string(&mut said).is_err() {
            return fail("stdin isn't text");
        }
    }
    let request = diagram::request(&format!("visualize {said}"))
        .or_else(|| diagram::request(&said))
        .unwrap_or(Request { kind: None, visualize: true, about: said.trim().to_string() });
    if request.about.is_empty() {
        return usage();
    }
    let start = Instant::now();
    let drawn = lf.diagram(&request).map_err(|e| e.0).and_then(|d| localflow_desktop::diagram::save(&d).map(|p| (d, p)));
    match drawn {
        Ok((d, drawn)) => {
            if let Some(why) = &d.fallback {
                eprintln!("  {} didn't draw it ({why}): a model on this computer did", lf.cfg.diagram_model);
            }
            println!("{}", d.mermaid.trim_end());
            eprintln!("  a {} in {} ms: {}", d.kind.name(), start.elapsed().as_millis(), drawn.png.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            let hint = match (lf.cfg.can_draw(), lf.cfg.haiku_diagrams()) {
                (true, _) => "",
                (false, true) => "", // the error says where the key goes
                (false, false) => " (localflow models download qwen3.5-2b)",
            };
            fail(format!("{e}{hint}"))
        }
    }
}

/// Each snippet as it's read: the trigger, then the text pasted for it.
fn snippets_command(lf: &Localflow) {
    let (snippets, path) = (lf.snippets(), lf.cfg.user_snippets());
    if snippets.is_empty() {
        println!("no snippets in {} (one a line: my email => jay@example.com)", path.display());
        return;
    }
    for s in snippets {
        println!("{}\n  {}\n", s.trigger, s.text.replace('\n', "\n  "));
    }
    println!("in {}", path.display());
}

/// Record `mic` (None: the default) for `seconds`, drawing the level on stderr.
fn record(mic: Option<&str>, seconds: f32) -> Result<Vec<i16>, String> {
    let loudness = Arc::new(AtomicU32::new(0));
    let shown = loudness.clone();
    let rec = Recorder::start_on(mic, move |l| shown.store(l.to_bits(), Ordering::Relaxed)).map_err(|e| e.to_string())?;
    if let Some(why) = rec.fallback() {
        eprintln!("  {why}; recording the default microphone");
    }
    let start = Instant::now();
    while start.elapsed().as_secs_f32() < seconds {
        let l = f32::from_bits(loudness.load(Ordering::Relaxed));
        let bar = "#".repeat((l * 30.0) as usize);
        eprint!("\r  listening {:4.1}s [{bar:<30}]", seconds - start.elapsed().as_secs_f32());
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!();
    Ok(rec.stop())
}

/// Speech to cleaned text, with the transcript, its segments and timings on stderr.
fn dictate(lf: &Localflow, samples: &[i16]) -> ExitCode {
    eprintln!("  {:.1}s of audio, level {:.2}", samples.len() as f32 / RATE as f32, level(samples));
    let start = Instant::now();
    let heard = match crate::dictation::transcribe(lf, samples) {
        Ok(heard) => heard,
        Err(e) => return fail(e),
    };
    eprintln!("  heard ({} ms):", start.elapsed().as_millis());
    for s in &heard.segments {
        eprintln!("    {:5.1}-{:5.1}s level {:.2}{} {}", s.start_s, s.end_s, s.level, if s.quiet { " quiet" } else { "" },
                  s.text.trim());
    }
    if heard.text.is_empty() {
        return fail("nothing heard");
    }
    let start = Instant::now();
    let input = heard.for_cleanup(true);
    let out = match lf.clean_detailed(&input, NO_FILES) {
        Ok(r) => {
            eprintln!("  cleaned ({} ms, {})", start.elapsed().as_millis(), r.verdict);
            r.text
        }
        Err(e) => {
            eprintln!("localflow: {e}; the raw transcript instead");
            heard.text
        }
    };
    println!("{out}");
    ExitCode::SUCCESS
}
