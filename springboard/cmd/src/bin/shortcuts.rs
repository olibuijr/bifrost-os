//! `shortcuts`: lists, shows and runs the shortcuts the Shortcuts app saves, runs droplets for dropped files and
//! writes their launchers. See `bifrost_cmd::shortcuts`.

use bifrost_cmd::die;
use bifrost_cmd::shortcuts::{self as cli, Cmd};
use springboard::shortcuts as model;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match cli::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("shortcuts", &e, 2),
    };
    if cmd == Cmd::Help {
        println!("{}", cli::USAGE);
        return;
    }
    let list = model::stored(&model::app_data_dir());
    match cmd {
        Cmd::Help => {}
        Cmd::List => print!("{}", cli::list_text(&list)),
        Cmd::View(what) => {
            let i = cli::resolve(&list, &what).unwrap_or_else(|e| die("shortcuts", &e, 1));
            print!("{}", cli::view_text(&list[i]));
        }
        Cmd::Launcher(what) => {
            let i = cli::resolve(&list, &what).unwrap_or_else(|e| die("shortcuts", &e, 1));
            let home = std::env::var("HOME").unwrap_or_else(|_| die("shortcuts", "HOME is not set", 1));
            match cli::write_launcher(std::path::Path::new(&home), i, &list[i]) {
                Ok(path) => println!("{}", path.display()),
                Err(e) => die("shortcuts", &e, 1),
            }
        }
        Cmd::Drop(what, files) => {
            let i = cli::resolve(&list, &what).unwrap_or_else(|e| die("shortcuts", &e, 1));
            let steps = cli::drop_shortcut(&list[i], &files).unwrap_or_else(|e| die("shortcuts", &e, 1));
            report(&steps);
        }
        Cmd::Run(what) => {
            let i = cli::resolve(&list, &what).unwrap_or_else(|e| die("shortcuts", &e, 1));
            report(&cli::run_shortcut(&list[i]));
        }
    }
}

/// Prints each step and exits with 1 when one did not run.
fn report(steps: &[cli::Step]) {
    let mut failed = false;
    for (k, step) in steps.iter().enumerate() {
        match &step.outcome {
            Ok(done) => println!("{}. {}: {done}", k + 1, step.line),
            Err(why) => {
                failed = true;
                println!("{}. {}: {why}", k + 1, step.line);
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}
