//! `arp`: shows the IPv4 neighbour (ARP) table. See `bifrost_cmd::neighbors`.

use bifrost_cmd::die;
use bifrost_cmd::neighbors::{self, Query, ARP_USAGE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (all, host) = match neighbors::parse_query(&args, ARP_USAGE) {
        Ok(Query::Help) => {
            println!("{ARP_USAGE}");
            return;
        }
        Ok(Query::List { all, host }) => (all, host),
        Err(e) => die("arp", &e, 1),
    };
    let text = neighbors::read_table("-4").unwrap_or_else(|e| die("arp", &e, 1));
    let table = neighbors::parse(&text);
    let selected = neighbors::select(&table, if all { None } else { host.as_deref() });
    if selected.is_empty() && !all {
        die("arp", &format!("no entry for {}", host.unwrap_or_default()), 1);
    }
    for n in selected {
        println!("{}", neighbors::arp_line(n));
    }
}
