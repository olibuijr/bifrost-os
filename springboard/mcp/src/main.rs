#![recursion_limit = "256"]
//! bifrost-mcp: a Model Context Protocol server (stdio, JSON-RPC 2.0, one message per line) that drives
//! BifrOSt PC from inside: the compositor over one persistent Wayland connection, keys through uinput,
//! the shell's control socket, every app's control socket, and the system itself. Run it on the machine
//! (as root), for example `ssh midget bifrost-mcp` as a Claude Code stdio server. Docs: docs/bifrost-mcp.md.

mod app;
mod browser;
mod term;
mod dev;
mod files;
mod keys;
mod lifecycle;
mod touch;
mod serve;
mod sys;
mod tools;
mod wl;

use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub const PROTOCOL: &str = "2025-06-18";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!("bifrost-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut ctx = tools::Ctx::new();
    // `bifrost-mcp call TOOL 'JSON'`: one tool call from a shell, for benchmarks and debugging
    if args.first().map(String::as_str) == Some("call") {
        let name = args.get(1).cloned().unwrap_or_default();
        let a: Value = args.get(2).map(|s| serde_json::from_str(s).expect("JSON arguments")).unwrap_or(json!({}));
        let t = std::time::Instant::now();
        let r = ctx.call(&name, &a);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        for c in r["content"].as_array().into_iter().flatten() {
            match c["type"].as_str() {
                Some("text") => println!("{}", c["text"].as_str().unwrap_or("")),
                Some("image") => {
                    let out = std::env::var("BIFROST_MCP_IMAGE").unwrap_or("/tmp/bifrost-mcp.png".into());
                    let _ = std::fs::write(&out, tools::b64_decode(c["data"].as_str().unwrap_or("")));
                    println!("[image -> {out}]");
                }
                _ => {}
            }
        }
        eprintln!("{name}: {ms:.1} ms{}", if r["isError"] == true { " (error)" } else { "" });
        std::process::exit(if r["isError"] == true { 1 } else { 0 });
    }
    if args.first().map(String::as_str) == Some("--serve") {
        serve::run(ctx);
        return;
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Some(r) = handle(&mut ctx, &line) {
            reply(&mut out, r);
        }
    }
}

/// One JSON-RPC message in, its reply out (None for notifications).
pub fn handle(ctx: &mut tools::Ctx, line: &str) -> Option<Value> {
    if line.trim().is_empty() {
        return None;
    }
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}})),
    };
    let id = msg.get("id").cloned()?; // notifications need no answer
    let method = msg["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "bifrost", "version": env!("CARGO_PKG_VERSION")},
            "instructions": tools::INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools::list()})),
        "tools/call" => {
            let p = &msg["params"];
            Ok(ctx.call(p["name"].as_str().unwrap_or(""), &p["arguments"]))
        }
        m => Err(json!({"code":-32601,"message":format!("method not found: {m}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc":"2.0","id":id,"result":r}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":e}),
    })
}

fn reply(out: &mut impl Write, v: Value) {
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}
