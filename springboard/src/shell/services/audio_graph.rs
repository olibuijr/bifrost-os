//! The `audio_graph` service: an app asks the shell to play the audio graph of a chart (`ui::chart_ax::ChartDescriptor::
//! audio_graph`, sent from the SDK's `audio_graph::play`). The tones are checked here and queued; the shell binary takes
//! the queue each loop (`take`), renders it with `shell::audio_graph::pcm` and plays it on the sound-effects role
//! (`SoundPlayer::audio_graph`). A new graph replaces one that has not started yet, so a user stepping fast through
//! series hears the last one, not a backlog.
//!
//! Methods: `play({"tones": [{"hz": f, "ms": n}, ...]})` answers `{"ms": total}`, the graph's length; `stop()` drops a
//! graph that is still queued and answers `true`. Limits: at most `MAX_TONES` tones, each `1..=MAX_TONE_MS` long, a
//! pitch of 0 (a rest) or `MIN_HZ..=MAX_HZ`, and at most `MAX_TOTAL_MS` in all. Only the app in front may play one, so a
//! background app cannot sound over the user's work.
use super::{Shell, SystemService};
use bifrost_core::json::Json;
use bifrost_ui::chart_ax::Tone;
use std::sync::Mutex;

pub const SERVICE: &str = "audio_graph";
pub const METHOD_PLAY: &str = "play";
pub const METHOD_STOP: &str = "stop";

pub const MAX_TONES: usize = 512;
pub const MAX_TONE_MS: u32 = 2000;
pub const MAX_TOTAL_MS: u32 = 60_000;
pub const MIN_HZ: f64 = 20.0;
pub const MAX_HZ: f64 = 8000.0;

static QUEUE: Mutex<Option<Vec<Tone>>> = Mutex::new(None);

/// The graph waiting to play, if any. The shell binary calls this once a loop.
pub fn take() -> Option<Vec<Tone>> {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn queue(tones: Option<Vec<Tone>>) {
    *QUEUE.lock().unwrap_or_else(|e| e.into_inner()) = tones;
}

/// Reads and checks the `tones` list of a `play` call. The timbre is the position of the series, which the shell does
/// not use yet, so it is 0.
pub fn parse_tones(params: &Json) -> Result<Vec<Tone>, String> {
    let Some(Json::Arr(list)) = params.get("tones") else {
        return Err("audio_graph: give the tones as {\"tones\": [{\"hz\": f, \"ms\": n}]}".to_string());
    };
    if list.is_empty() {
        return Err("audio_graph: the graph has no tones".to_string());
    }
    if list.len() > MAX_TONES {
        return Err(format!("audio_graph: at most {MAX_TONES} tones"));
    }
    let mut out = Vec::with_capacity(list.len());
    let mut total: u32 = 0;
    for (i, t) in list.iter().enumerate() {
        let (Some(Json::Num(hz)), Some(Json::Num(ms))) = (t.get("hz"), t.get("ms")) else {
            return Err(format!("audio_graph: tone {i} needs hz and ms"));
        };
        let (hz, ms) = (*hz, *ms);
        if !hz.is_finite() || (hz != 0.0 && !(MIN_HZ..=MAX_HZ).contains(&hz)) {
            return Err(format!("audio_graph: tone {i} has pitch {hz}; use 0 for a rest or {MIN_HZ} to {MAX_HZ} Hz"));
        }
        if !ms.is_finite() || ms < 1.0 || ms > MAX_TONE_MS as f64 {
            return Err(format!("audio_graph: tone {i} lasts {ms} ms; use 1 to {MAX_TONE_MS}"));
        }
        let millis = ms.round() as u32;
        total += millis;
        if total > MAX_TOTAL_MS {
            return Err(format!("audio_graph: the graph is longer than {MAX_TOTAL_MS} ms"));
        }
        out.push(Tone { hz, millis, timbre: 0 });
    }
    Ok(out)
}

#[derive(Default)]
pub struct AudioGraph;

impl SystemService for AudioGraph {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        match method {
            METHOD_PLAY => {
                // the app's id is its manifest id, as the service call carries it (`apphost`, `ServiceCall`)
                let front = shell.ext_fg.and_then(|n| crate::shell::apphost::installed().get(n as usize).map(|a| a.manifest.id.clone()));
                if !app_id.is_empty() && front.as_deref() != Some(app_id) {
                    return Err("audio_graph: only the app in front can play an audio graph".to_string());
                }
                let tones = parse_tones(params)?;
                let total: u32 = tones.iter().map(|t| t.millis).sum();
                queue(Some(tones));
                Ok(Json::Obj(vec![("ms".to_string(), Json::Num(total as f64))]))
            }
            METHOD_STOP => {
                queue(None);
                Ok(Json::Bool(true))
            }
            _ => Err(format!("audio_graph has no method {method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tones(list: &[(f64, f64)]) -> Json {
        let arr = list.iter().map(|(hz, ms)| Json::Obj(vec![("hz".into(), Json::Num(*hz)), ("ms".into(), Json::Num(*ms))])).collect();
        Json::Obj(vec![("tones".into(), Json::Arr(arr))])
    }

    #[test]
    fn tones_and_rests_are_read_in_order() {
        let t = parse_tones(&tones(&[(440.0, 250.0), (0.0, 100.4), (1760.0, 800.0)])).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].hz, t[0].millis), (440.0, 250));
        assert_eq!((t[1].hz, t[1].millis), (0.0, 100), "a rest keeps its length");
        assert_eq!(t[2].millis, 800);
    }

    #[test]
    fn bad_graphs_are_refused() {
        assert!(parse_tones(&Json::Null).is_err());
        assert!(parse_tones(&tones(&[])).is_err(), "no tones");
        assert!(parse_tones(&tones(&[(5.0, 100.0)])).is_err(), "below hearing");
        assert!(parse_tones(&tones(&[(440.0, 0.0)])).is_err(), "no length");
        assert!(parse_tones(&tones(&[(440.0, 5000.0)])).is_err(), "one tone too long");
        assert!(parse_tones(&tones(&[(f64::NAN, 100.0)])).is_err());
        let many: Vec<(f64, f64)> = vec![(440.0, 1.0); MAX_TONES + 1];
        assert!(parse_tones(&tones(&many)).is_err(), "too many tones");
        let long: Vec<(f64, f64)> = vec![(440.0, 2000.0); 31];
        assert!(parse_tones(&tones(&long)).is_err(), "longer than a minute in all");
    }

    #[test]
    fn a_new_graph_replaces_a_waiting_one_and_stop_drops_it() {
        queue(Some(vec![Tone { hz: 440.0, millis: 10, timbre: 0 }]));
        queue(Some(vec![Tone { hz: 880.0, millis: 10, timbre: 0 }]));
        assert_eq!(take().map(|t| t[0].hz), Some(880.0));
        assert_eq!(take(), None);
        queue(Some(vec![Tone { hz: 440.0, millis: 10, timbre: 0 }]));
        queue(None);
        assert_eq!(take(), None);
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(AudioGraph.name(), "audio_graph");
    }
}
