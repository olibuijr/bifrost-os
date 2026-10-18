//! `automator`: runs a workflow from the command line. A workflow is either a `.automator` file saved by the Automator
//! app (its model and runner are the app's own modules, compiled in below, so the command and the window run the same
//! steps) or a shortcut saved in the Shortcuts app, named by its name or id (run as `shortcuts run` does).
//! See `bifrost_cmd::shortcuts::parse_automator` for the command line.
#![allow(dead_code)]

// The Automator app's modules, shared with the window so a workflow behaves the same in both.
#[path = "../../../apps/automator-desktop/src/actions.rs"]
mod actions;
#[path = "../../../apps/automator-desktop/src/imageio.rs"]
mod imageio;
#[path = "../../../apps/automator-desktop/src/model.rs"]
mod model;

use bifrost_cmd::die;
use bifrost_cmd::shortcuts::{self as cli, Cmd};
use springboard::shortcuts as model_shortcuts;

/// Runs the steps of a `.automator` file, printing the log lines. Returns the exit status.
fn run_workflow_file(path: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => die("automator", &format!("{path}: {e}"), 1),
    };
    let workflow = match model::Workflow::from_file(&text) {
        Ok(w) => w,
        Err(e) => die("automator", &format!("{path}: {e}"), 1),
    };
    let mut failed = false;
    let result = model::execute(&workflow.steps, &mut |line| {
        failed |= line.error;
        println!("{}", line.text);
    });
    match result {
        Ok(_) if !failed => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("automator: {e}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_automator(&args) {
        Err(e) => die("automator", &e, 1),
        Ok(Cmd::Help) => println!("{}", cli::AUTOMATOR_USAGE),
        Ok(Cmd::Run(what)) => {
            if std::path::Path::new(&what).is_file() {
                std::process::exit(run_workflow_file(&what));
            }
            let list = model_shortcuts::stored(&model_shortcuts::app_data_dir());
            std::process::exit(cli::run_and_print(&list, &what));
        }
        Ok(_) => unreachable!("parse_automator returns Help or Run"),
    }
}
