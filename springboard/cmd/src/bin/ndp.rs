//! `ndp`: shows the IPv6 neighbour table. See `bifrost_cmd::neighbors`.

use bifrost_cmd::die;
use bifrost_cmd::neighbors::{self, Query, NDP_USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (all, host) = match neighbors::parse_query(&args, NDP_USAGE) {
        Ok(Query::Help) => {
            println!("{NDP_USAGE}");
            return;
        }
        Ok(Query::List { all, host }) => (all, host),
        Err(e) => die("ndp", &e, 1),
    };
    let text = neighbors::read_table("-6").unwrap_or_else(|e| die("ndp", &e, 1));
    let table = neighbors::parse(&text);
    let selected = neighbors::select(&table, if all { None } else { host.as_deref() });
    if selected.is_empty() && !all {
        die("ndp", &format!("no entry for {}", host.unwrap_or_default()), 1);
    }
    println!("{}", neighbors::NDP_HEADER);
    for n in selected {
        println!("{}", neighbors::ndp_line(n));
    }
}
