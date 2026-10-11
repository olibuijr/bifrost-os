//! `daw-render <project.json> <out.wav> [--lab] [--sr 48000] [--f32] [--plugins DIR]`
//! Renders a song offline and, with `--lab`, prints the listening metrics and the target check.
use daw::{lab, model::Project, render, wav};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = vec![];
    let mut lab_on = false;
    let mut f32_out = false;
    let mut sr = render::DEFAULT_SR;
    let mut plugins = daw::assets::asset_root().join("plugins");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lab" => lab_on = true,
            "--f32" => f32_out = true,
            "--sr" => {
                i += 1;
                sr = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(sr);
            }
            "--plugins" => {
                i += 1;
                plugins = PathBuf::from(args.get(i).cloned().unwrap_or_default());
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    let [src, out] = positional.as_slice() else {
        eprintln!("usage: daw-render <project.json> <out.wav> [--lab] [--sr N] [--f32] [--plugins DIR]");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(src).unwrap_or_else(|e| {
        eprintln!("daw-render: {src}: {e}");
        std::process::exit(1)
    });
    let project = Project::from_json(&text).unwrap_or_else(|e| {
        eprintln!("daw-render: {src}: {e}");
        std::process::exit(1)
    });
    let opts = render::RenderOpts { sample_rate: sr, plugins, ..Default::default() };
    let started = std::time::Instant::now();
    let rendered = render::render(&project, &opts);
    let bytes = if f32_out {
        wav::encode_f32(&rendered.left, &rendered.right, rendered.sample_rate)
    } else {
        wav::encode16(&rendered.left, &rendered.right, rendered.sample_rate, opts.seed)
    };
    wav::write_file(&PathBuf::from(out), &bytes).unwrap_or_else(|e| {
        eprintln!("daw-render: {out}: {e}");
        std::process::exit(1)
    });
    println!("rendered {:.1} s in {:.1} s -> {out}", rendered.left.len() as f64 / sr as f64, started.elapsed().as_secs_f64());
    if !rendered.missing.is_empty() {
        println!("missing: {}", rendered.missing.join(", "));
    }
    if lab_on {
        let m = lab::measure(&rendered.left, &rendered.right, rendered.sample_rate);
        println!(
            "LUFS {:.1}  TP {:.1} dBTP  clipped {}  crest {:.1} dB  bands sub/bass/lowmid/high {:.0}/{:.0}/{:.0}/{:.0} %",
            m.lufs, m.true_peak_db, m.clipped, m.crest_db, m.bands[0], m.bands[1], m.bands[2], m.bands[3]
        );
        for p in m.check_targets() {
            println!("target: {p}");
        }
    }
}
