//! `automator`: runs a saved workflow from the command line. A workflow here is a shortcut saved in the Shortcuts app;
//! the steps run exactly as `shortcuts run` runs them. See `bifrost_cmd::shortcuts::parse_automator`.

use bifrost_cmd::die;
use bifrost_cmd::shortcuts::{self as cli, Cmd};
use springboard::shortcuts as model;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse_automator(&args) {
        Err(e) => die("automator", &e, 1),
        Ok(Cmd::Help) => println!("{}", cli::AUTOMATOR_USAGE),
        Ok(Cmd::Run(what)) => {
            let list = model::stored(&model::app_data_dir());
            std::process::exit(cli::run_and_print(&list, &what));
        }
        Ok(_) => unreachable!("parse_automator returns Help or Run"),
    }
}
