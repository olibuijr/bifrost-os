//! `vis`: encodes files or standard input as printable escape sequences. See `bifrost_cmd::vis`.

use bifrost_cmd::die;
use bifrost_cmd::vis::{self, VIS_USAGE};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match vis::parse_args(&args) {
        Ok(cli) => cli,
        Err(e) => die("vis", &e, 1),
    };
    if cli.help {
        println!("{VIS_USAGE}");
        return;
    }
    let names = if cli.files.is_empty() { vec!["-".to_string()] } else { cli.files };
    let mut stdout = std::io::stdout().lock();
    for name in &names {
        let input = match vis::read_input(name) {
            Ok(input) => input,
            Err(e) => die("vis", &e, 1),
        };
        if let Err(e) = stdout.write_all(&vis::encode(&input, &cli.opts)) {
            die("vis", &e.to_string(), 1);
        }
    }
}
