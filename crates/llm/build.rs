//! Builds native/ (lf.cpp and llama.cpp, always optimized: a debug llama.cpp
//! is unusably slow) into one shared library, and leaves it zstd-compressed in
//! OUT_DIR for src/lib.rs to embed.

use std::env;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let llama = dir.join("llama.cpp");
    if !llama.join("CMakeLists.txt").exists() {
        panic!("llama.cpp is missing: run `git submodule update --init --depth 1`");
    }
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=llama.cpp/include/llama.h");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    let mut cfg = cmake::Config::new(dir.join("native"));
    cfg.profile("Release").define("LLAMA_DIR", &llama);
    // The same CPU baseline as whisper.cpp's (.cargo/config.toml): not the build machine's.
    for var in ["GGML_NATIVE", "GGML_AVX", "GGML_AVX2", "GGML_FMA", "GGML_F16C"] {
        println!("cargo:rerun-if-env-changed={var}");
        if let Ok(v) = env::var(var) {
            cfg.define(var, v);
        }
    }
    if env::var("CARGO_FEATURE_VULKAN").is_ok() {
        cfg.define("GGML_VULKAN", "ON");
    }
    if os == "macos" {
        cfg.define("GGML_METAL", "ON").define("GGML_METAL_EMBED_LIBRARY", "ON");
    }
    let out = cfg.build();

    let file = match os.as_str() {
        "windows" => out.join("bin/localflow_llm.dll"),
        "macos" => out.join("lib/liblocalflow_llm.dylib"),
        _ => out.join("lib/liblocalflow_llm.so"),
    };
    let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    // Mostly GPU shaders, which compress well.
    let packed = zstd::encode_all(&bytes[..], 19).unwrap();
    std::fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("llm.zst"), packed).unwrap();
}
