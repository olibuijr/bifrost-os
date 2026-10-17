//! `unvis`: decodes escape sequences made by `vis` back into bytes. See `bifrost_cmd::vis`.

use bifrost_cmd::die;
use bifrost_cmd::vis::{self, UNVIS_USAGE};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match vis::parse_args(&args) {
        Ok(cli) => cli,
        Err(e) => die("unvis", &e, 1),
    };
    if cli.help {
        println!("{UNVIS_USAGE}");
        return;
    }
    let names = if cli.files.is_empty() { vec!["-".to_string()] } else { cli.files };
    let mut stdout = std::io::stdout().lock();
    for name in &names {
        let input = match vis::read_input(name) {
            Ok(input) => input,
            Err(e) => die("unvis", &e, 1),
        };
        let output = match vis::decode(&input, &cli.opts) {
            Ok(output) => output,
            Err(e) => die("unvis", &e, 1),
        };
        if let Err(e) = stdout.write_all(&output) {
            die("unvis", &e.to_string(), 1);
        }
    }
}
