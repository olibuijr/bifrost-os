//! `bifrost-log`: the `log` CLI (Unified Logging, docs/bifrost-ui.md).
//!
//! bifrost-log show   [--predicate P] [--subsystem S] [--category C] [--process N] [--level L] [--last 1h] [--limit N] [--contains T]
//! bifrost-log stream [same filters]
//! bifrost-log config --subsystem S [--level debug|info|notice|error|fault] [--private on|off]
//! bifrost-log subsystems
use bifrost_app::log::{self, format_entry, Level, LogStore, Query};

fn usage() -> ! {
    eprintln!("usage: bifrost-log show|stream [--predicate P] [--subsystem S] [--category C] [--process N] [--level L] [--last 1h] [--limit N] [--contains T]\n       bifrost-log config --subsystem S [--level L] [--private on|off]\n       bifrost-log subsystems");
    std::process::exit(2)
}

fn flags(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let k = a.strip_prefix("--").ok_or_else(|| format!("unexpected argument {a:?}"))?;
        out.push((k.to_string(), it.next().ok_or_else(|| format!("--{k} needs a value"))?.clone()));
    }
    Ok(out)
}

fn query(fl: &[(String, String)]) -> Result<Query, String> {
    let mut q = Query::default();
    for (k, v) in fl {
        match k.as_str() {
            "predicate" => q = Query { limit: q.limit, ..Query::parse_predicate(v)? },
            "subsystem" => q.subsystem = Some(v.clone()),
            "category" => q.category = Some(v.clone()),
            "process" => q.process = Some(v.clone()),
            "contains" => q.contains = Some(v.clone()),
            "level" => q.min_level = Some(Level::parse(v).ok_or_else(|| format!("unknown level {v:?}"))?),
            "last" => q.last = Some(log::parse_duration(v).ok_or_else(|| format!("bad duration {v:?} (use 30s, 10m, 1h, 2d)"))?),
            "limit" => q.limit = Some(v.parse().map_err(|_| format!("bad limit {v:?}"))?),
            _ => return Err(format!("unknown option --{k}")),
        }
    }
    Ok(q)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    let fl = flags(&args[1..])?;
    match cmd.as_str() {
        "show" => {
            for e in LogStore::system().entries(&query(&fl)?).map_err(|e| e.to_string())? {
                println!("{}", format_entry(&e));
            }
        }
        "stream" => {
            LogStore::system()
                .stream(&query(&fl)?, |e| {
                    println!("{}", format_entry(&e));
                    true
                })
                .map_err(|e| e.to_string())?;
        }
        "config" => {
            let get = |k: &str| fl.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
            let sub = get("subsystem").ok_or("config needs --subsystem")?;
            if !log::valid_subsystem(sub) {
                return Err(format!("invalid subsystem {sub:?}"));
            }
            let level = get("level").map(|v| Level::parse(v).ok_or_else(|| format!("unknown level {v:?}"))).transpose()?;
            let private = get("private").map(|v| match v {
                "on" => Ok(true),
                "off" => Ok(false),
                _ => Err(format!("--private takes on or off, not {v:?}")),
            }).transpose()?;
            if level.is_none() && private.is_none() {
                let (l, p) = log::effective(&log::read_merged(), sub);
                println!("{sub}: level {} private {}", l.name(), if p { "on" } else { "off" });
                return Ok(());
            }
            let mut ov = log::read_overrides();
            log::set_override(&mut ov, sub, level, private);
            log::write_overrides(&ov).map_err(|e| format!("cannot write {}: {e} (run as root)", log::OVERRIDES_PATH))?;
            let (l, p) = log::effective(&log::read_merged(), sub);
            println!("{sub}: level {} private {} (apps pick it up within a second)", l.name(), if p { "on" } else { "off" });
        }
        "subsystems" => {
            for s in log::read_merged() {
                println!("{}\t{}\t{}\tprivate {}", s.id, s.level.unwrap_or(Level::Notice).name(), s.categories.join(","), if s.private == Some(true) { "on" } else { "off" });
            }
        }
        _ => usage(),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("bifrost-log: {e}");
        std::process::exit(1);
    }
}
