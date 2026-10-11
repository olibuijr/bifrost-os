//! `daw-ai <project.json> "<prompt>" [--model NAME] [--dry-run]`
//! Asks the AI co-producer for changes, applies the supported operations and saves the song (atomically).
use daw::{ai, lab, model::Project, render, store};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = vec![];
    let mut model = None;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => dry = true,
            "--model" => {
                i += 1;
                model = args.get(i).cloned();
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    let [path, prompt] = positional.as_slice() else {
        eprintln!("usage: daw-ai <project.json> \"<prompt>\" [--model NAME] [--dry-run]");
        std::process::exit(2);
    };
    let path = PathBuf::from(path);
    let mut project = store::load(&path).unwrap_or_else(|e| {
        eprintln!("daw-ai: {e}");
        std::process::exit(1)
    });
    let mut reply = ai::ask(&project, prompt, model.as_deref()).unwrap_or_else(|e| {
        eprintln!("daw-ai: {e}");
        std::process::exit(1)
    });
    println!("{}", reply.text);
    apply_and_report(&mut project, &reply);
    // ears: render, measure against the listening targets, and correct (at most two passes)
    for pass in 1..=2 {
        let rendered = render::render(&project, &render::RenderOpts::default());
        let m = lab::measure(&rendered.left, &rendered.right, rendered.sample_rate);
        let problems = m.check_targets();
        println!(
            "heard (pass {pass}): {:.1} LUFS, TP {:.1} dBTP, clipped {}, sub+bass {:.0}%",
            m.lufs,
            m.true_peak_db,
            m.clipped,
            m.bands[0] + m.bands[1]
        );
        if problems.is_empty() {
            break;
        }
        let report = format!(
            "Listening report: {}. Correct the mix with set_mix / set_param (volumes, pan, effect params) and reply with ops only changes that fix these problems.",
            problems.join("; ")
        );
        reply = ai::ask(&project, &report, model.as_deref()).unwrap_or_else(|e| {
            eprintln!("daw-ai: {e}");
            std::process::exit(1)
        });
        println!("{}", reply.text);
        apply_and_report(&mut project, &reply);
    }
    if dry {
        return;
    }
    let problems = project.validate();
    if !problems.is_empty() {
        eprintln!("not saved, the song would be invalid: {}", problems.join("; "));
        std::process::exit(1);
    }
    store::write_atomic(&path, &project.to_json()).unwrap_or_else(|e| {
        eprintln!("daw-ai: {e}");
        std::process::exit(1)
    });
    println!("saved {}", path.display());
}

fn apply_and_report(project: &mut Project, reply: &ai::Reply) {
    let (applied, skipped) = ai::apply_ops(project, &reply.ops);
    println!("applied {applied} of {} operations", reply.ops.len());
    if !skipped.is_empty() {
        println!("not applied: {}", skipped.join(", "));
    }
}
