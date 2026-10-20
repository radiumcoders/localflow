//! localflow's build and test tasks, in Rust like the rest:
//!
//!     cargo xtask package --version X.Y.Z [--features vulkan]   # the installer, in build/
//!     cargo xtask e2e                                           # dictate end to end with the release build
//!     cargo xtask installed build/localflow-…                   # install it, check it, remove it

mod check;
mod fake_api;
mod icon;
mod keys;
mod package;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::LazyLock;

/// The repository.
static ROOT: LazyLock<PathBuf> = LazyLock::new(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap());

/// Run a program, failing on a non-zero exit.
fn run(program: &str, args: &[&str]) -> Result<(), String> {
    run_env(program, args, &[])
}

fn run_env(program: &str, args: &[&str], env: &[(&str, String)]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .current_dir(&*ROOT)
        .status()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if !status.success() {
        return Err(format!("{program} {} exited {status}", args.join(" ")));
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("package") => package::main(rest),
        Some("e2e") => check::e2e(rest),
        Some("installed") => check::installed(rest),
        _ => Err("cargo xtask package --version X.Y.Z [--features vulkan] | e2e | installed INSTALLER".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}
