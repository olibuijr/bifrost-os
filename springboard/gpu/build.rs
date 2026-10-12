//! Compiles every GLSL file in `shaders/` (`.vert`, `.frag`, `.comp`) to SPIR-V with `glslc` and writes
//! `$OUT_DIR/shaders.rs` embedding them. Without `glslc` the table is empty and `shader()` returns `None`, so the
//! crate still builds on machines without the Vulkan SDK (the GPU path then reports `Unsupported`).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("shaders");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=shaders");
    let mut table = String::from("pub static SHADERS: &[(&str, &[u8])] = &[\n");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    entries.sort();
    for src in entries {
        let Some(ext) = src.extension().and_then(|e| e.to_str()) else { continue };
        if !matches!(ext, "vert" | "frag" | "comp") {
            continue;
        }
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        println!("cargo:rerun-if-changed={}", src.display());
        let spv = out.join(format!("{name}.spv"));
        let ok = Command::new("glslc")
            .args(["--target-env=vulkan1.3", "-O", "-I"])
            .arg(&dir)
            .arg(&src)
            .arg("-o")
            .arg(&spv)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            println!("cargo:warning=glslc failed or is missing for {name}; the shader is left out");
            continue;
        }
        let _ = writeln!(table, "    ({name:?}, include_bytes!({:?})),", spv.display().to_string());
    }
    table.push_str("];\n");
    std::fs::write(out.join("shaders.rs"), table).unwrap();
}
