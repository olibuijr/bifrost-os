//! The shared output format: every subcommand prints one JSON object (docs/audio/19-bench-instrument.md).
//! A tiny writer, since no JSON crate is available offline.

use crate::stats::Summary;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Uint(u64),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }

    /// An object from `(key, value)` pairs, in order.
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Uint(u) => out.push_str(&u.to_string()),
            // JSON has no NaN or infinity.
            Json::Num(f) if f.is_finite() => out.push_str(&f.to_string()),
            Json::Num(_) => out.push_str("null"),
            Json::Str(s) => write_str(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    it.write(out);
                }
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One benchmark result in the shared shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub bench: String,
    pub host: String,
    pub driver: String,
    pub rate: u32,
    pub quantum: u32,
    /// `stats.n` is the `samples` field; the statistics are in milliseconds.
    pub stats: Summary,
    /// What the HAL latency properties predict, in milliseconds.
    pub reported_ms: Option<f64>,
    pub xruns: u64,
    pub dropped: u64,
    pub extra: Vec<(String, Json)>,
}

impl Report {
    pub fn to_json(&self) -> Json {
        let s = &self.stats;
        Json::obj(vec![
            ("bench", Json::str(&self.bench)),
            ("host", Json::str(&self.host)),
            ("driver", Json::str(&self.driver)),
            ("rate", Json::Uint(self.rate as u64)),
            ("quantum", Json::Uint(self.quantum as u64)),
            ("samples", Json::Uint(s.n as u64)),
            ("p50_ms", Json::Num(s.p50)),
            ("p99_ms", Json::Num(s.p99)),
            ("max_ms", Json::Num(s.max)),
            ("min_ms", Json::Num(s.min)),
            ("jitter_ms", Json::Num(s.jitter())),
            ("reported_ms", self.reported_ms.map_or(Json::Null, Json::Num)),
            ("xruns", Json::Uint(self.xruns)),
            ("dropped", Json::Uint(self.dropped)),
            ("extra", Json::Obj(self.extra.clone())),
        ])
    }

    /// A short human-readable summary (the default output without `--json`).
    pub fn to_text(&self) -> String {
        let s = &self.stats;
        let reported = self.reported_ms.map_or("n/a".to_string(), |r| format!("{r:.3} ms"));
        format!(
            "{} on {} ({} Hz, quantum {}): {} samples, p50 {:.3} ms, p99 {:.3} ms, min {:.3} ms, max {:.3} ms, jitter {:.3} ms, reported {}, {} xruns, {} dropped",
            self.bench, self.driver, self.rate, self.quantum, s.n, s.p50, s.p99, s.min, s.max, s.jitter(), reported, self.xruns, self.dropped
        )
    }
}

/// The machine's host name (`/proc/sys/kernel/hostname`, else `$HOSTNAME`, else "unknown").
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_and_escapes() {
        assert_eq!(Json::Null.to_string(), "null");
        assert_eq!(Json::Bool(true).to_string(), "true");
        assert_eq!(Json::Uint(7).to_string(), "7");
        assert_eq!(Json::Num(2.5).to_string(), "2.5");
        assert_eq!(Json::Num(2.0).to_string(), "2");
        assert_eq!(Json::Num(f64::NAN).to_string(), "null");
        assert_eq!(Json::Num(f64::INFINITY).to_string(), "null");
        assert_eq!(Json::str("a\"b\\c\nd\u{1}").to_string(), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn nested() {
        let j = Json::obj(vec![("a", Json::Arr(vec![Json::Uint(1), Json::Num(0.5)])), ("b", Json::obj(vec![]))]);
        assert_eq!(j.to_string(), "{\"a\":[1,0.5],\"b\":{}}");
    }

    #[test]
    fn report_has_the_shared_shape() {
        let r = Report {
            bench: "midi-latency".into(),
            host: "h".into(),
            driver: "null".into(),
            rate: 48_000,
            quantum: 64,
            stats: Summary { n: 3, min: 1.5, max: 3.5, p50: 2.5, p99: 3.5 },
            reported_ms: None,
            xruns: 0,
            dropped: 0,
            extra: vec![("late".into(), Json::Uint(2))],
        };
        assert_eq!(
            r.to_json().to_string(),
            "{\"bench\":\"midi-latency\",\"host\":\"h\",\"driver\":\"null\",\"rate\":48000,\"quantum\":64,\"samples\":3,\
\"p50_ms\":2.5,\"p99_ms\":3.5,\"max_ms\":3.5,\"min_ms\":1.5,\"jitter_ms\":1,\"reported_ms\":null,\"xruns\":0,\"dropped\":0,\
\"extra\":{\"late\":2}}"
        );
        let mut r2 = r.clone();
        r2.reported_ms = Some(2.0);
        assert!(r2.to_json().to_string().contains("\"reported_ms\":2,"));
        assert!(r2.to_text().contains("reported 2.000 ms"));
    }

    #[test]
    fn hostname_is_not_empty() {
        assert!(!hostname().is_empty());
    }
}
