//! Compiles every GLSL file in `shaders/` (`.vert`, `.frag`, `.comp`) to SPIR-V with `glslc` and writes
//! `$OUT_DIR/shaders.rs` embedding them. Without `glslc` the table is empty and `shader()` returns `None`, so the
//! crate still builds on machines without the Vulkan SDK (the GPU path then reports `Unsupported`).
//!
//! Failures never stop the build: a shader that does not compile is left out of the table and every line of its
//! compiler output becomes a `cargo:warning` that names the file. Include files (any other extension, such as
//! `.glsl`) are tracked, so editing one rebuilds the shaders that may use it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Stage extensions that are compiled to SPIR-V; everything else in `shaders/` is an include file.
const STAGES: [&str; 3] = ["vert", "frag", "comp"];

/// At most this many compiler output lines are echoed per failing shader, so a broken include does not flood the log.
const MAX_ERROR_LINES: usize = 8;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into())).join("shaders");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into()));
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-env-changed=PATH");

    let entries = list_shader_dir(&dir);
    for path in &entries {
        // Every file, include or stage, is an input: an edited include must rebuild the shaders that use it.
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let mut table = String::from("pub static SHADERS: &[(&str, &[u8])] = &[\n");
    let stages: Vec<&PathBuf> = entries.iter().filter(|p| is_stage(p)).collect();
    if !stages.is_empty() && !glslc_available() {
        println!("cargo:warning=glslc not found in PATH: {} shader(s) not built (install the Vulkan SDK or shaderc)", stages.len());
    } else {
        for src in stages {
            compile(src, &dir, &out, &mut table);
        }
    }
    table.push_str("];\n");
    if let Err(e) = std::fs::write(out.join("shaders.rs"), table) {
        // Without the table the crate cannot compile; this is the one failure that must stop the build.
        panic!("bifrost-gpu build.rs: cannot write shaders.rs: {e}");
    }
}

/// The regular files in `dir`, sorted, hidden files skipped. A missing directory is an empty set.
fn list_shader_dir(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect(),
        Err(e) => {
            println!("cargo:warning=bifrost-gpu: cannot read {}: {e}; no shaders built", dir.display());
            Vec::new()
        }
    };
    entries.sort();
    entries
}

fn is_stage(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| STAGES.contains(&e))
}

fn glslc_available() -> bool {
    // A missing binary is a spawn error; any other exit status still means glslc exists.
    Command::new("glslc").arg("--version").output().is_ok()
}

/// Compiles one stage file. On success its SPIR-V is embedded; on failure a warning names the file and the first
/// compiler messages, and the shader is left out.
fn compile(src: &Path, dir: &Path, out: &Path, table: &mut String) {
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let spv = out.join(format!("{name}.spv"));
    // A stale artifact from an earlier build must never be embedded for a shader that now fails.
    let _ = std::fs::remove_file(&spv);

    let result = Command::new("glslc")
        .args(["--target-env=vulkan1.3", "-O", "-I"])
        .arg(dir)
        .arg(src)
        .arg("-o")
        .arg(&spv)
        .output();
    let output = match result {
        Ok(o) => o,
        Err(e) => {
            println!("cargo:warning=glslc could not run for {name}: {e}; the shader is left out");
            return;
        }
    };
    if !output.status.success() || !spv.is_file() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut lines = stderr.lines().filter(|l| !l.trim().is_empty());
        let head = lines.next().unwrap_or("no compiler output");
        println!("cargo:warning=glslc failed for {name} ({}): {head}; the shader is left out", src.display());
        for line in lines.take(MAX_ERROR_LINES - 1) {
            println!("cargo:warning=  {name}: {line}");
        }
        return;
    }
    let _ = writeln!(table, "    ({name:?}, include_bytes!({:?})),", spv.display().to_string());
}
