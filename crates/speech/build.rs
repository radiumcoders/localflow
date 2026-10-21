//! macOS: whisper.cpp's Metal code checks the system version (`@available`),
//! which calls into clang's runtime library; Rust links without it, so the
//! app failed to link (`___isPlatformVersionAtLeast`).

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "clang".into());
    let dir = Command::new(&cc).arg("--print-runtime-dir").output().ok().filter(|o| o.status.success());
    let dir = dir.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if std::path::Path::new(&dir).join("libclang_rt.osx.a").is_file() {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    } else {
        println!("cargo:warning=no libclang_rt.osx.a from `{cc} --print-runtime-dir` ({dir:?}): linking may fail");
    }
}
