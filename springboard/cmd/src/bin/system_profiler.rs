//! `system_profiler`: reports the hardware and software of this computer as text or JSON. See `bifrost_cmd::profile`.

use bifrost_cmd::die;
use bifrost_cmd::profile::{self, DataType, Node};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match profile::parse(&args) {
        Ok(options) => options,
        Err(e) => die("system_profiler", &e, 1),
    };
    if options.help {
        println!("{}", profile::USAGE);
        return;
    }
    if options.list {
        for t in profile::ALL {
            println!("{}", t.id());
        }
        return;
    }
    let types: Vec<DataType> = if options.types.is_empty() { profile::ALL.to_vec() } else { options.types.clone() };
    let collected = profile::collect();
    let sections: Vec<(DataType, Vec<Node>)> = types.iter().map(|t| (*t, profile::items(*t, &collected.report, &collected.audio, options.detail))).collect();
    let text = if options.json { profile::render_json(&sections) } else { profile::render_text(&sections) };
    print!("{text}");
}
