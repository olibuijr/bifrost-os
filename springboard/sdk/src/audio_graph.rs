//! Audio graphs for apps: play a chart's data as a run of tones, pitch following the value, so a listener hears its
//! shape. Over the shell's `audio_graph` service (`springboard/src/shell/services/audio_graph.rs`).
//!
//! ```ignore
//! // a chart the app already describes for the screen reader:
//! let chart = ChartDescriptor::new("Rainfall", "Monthly rain", x, Some(y), series);
//! // the user chose "Play Audio Graph" (a custom action or a menu item):
//! audio_graph::play(ctx, &chart, 0);
//! // in App::service_reply: audio_graph::length_from_reply(result) gives the graph's length in ms
//! ```
//!
//! The tones come from `ChartDescriptor::audio_graph` (`bifrost_ui::chart_ax`): a logarithmic pitch scale over two
//! octaves, tone length from the first extra value, a rest where a point has no value. The reference renders a chart's
//! audio graph from its data series descriptor in the same way and plays it from the screen reader; here the app asks for
//! it, only while it is in front. A new graph replaces one that has not started.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
pub use bifrost_ui::chart_ax::{ChartDescriptor, Tone};

pub const SERVICE: &str = "audio_graph";
pub const METHOD_PLAY: &str = "play";
pub const METHOD_STOP: &str = "stop";

/// The `play` params for `tones`: `{"tones": [{"hz": f, "ms": n}, ...]}`.
pub fn tones_json(tones: &[Tone]) -> Json {
    let list = tones
        .iter()
        .map(|t| Json::Obj(vec![("hz".to_string(), Json::Num(t.hz)), ("ms".to_string(), Json::Num(t.millis.max(1) as f64))]))
        .collect();
    Json::Obj(vec![("tones".to_string(), Json::Arr(list))])
}

/// Plays the audio graph of series `series` of `chart`. Returns the request id, or `None` when that series has no
/// points (nothing is sent).
pub fn play(ctx: &Ctx, chart: &ChartDescriptor, series: usize) -> Option<u32> {
    let tones = chart.audio_graph(series);
    (!tones.is_empty()).then(|| play_tones(ctx, &tones))
}

/// Plays `tones` as they are. Returns the request id.
pub fn play_tones(ctx: &Ctx, tones: &[Tone]) -> u32 {
    service::call(ctx, SERVICE, METHOD_PLAY, &tones_json(tones))
}

/// Drops a graph that has not started yet. Returns the request id.
pub fn stop(ctx: &Ctx) -> u32 {
    service::call(ctx, SERVICE, METHOD_STOP, &Json::Null)
}

/// Decodes a `play` reply: the graph's length in milliseconds, or the shell's reason for refusing it.
pub fn length_from_reply(result: &Result<Json, String>) -> Result<u32, String> {
    match result {
        Ok(body) => match body.get("ms") {
            Some(Json::Num(ms)) if ms.is_finite() && *ms >= 0.0 => Ok(ms.round() as u32),
            _ => Err("audio_graph: the reply has no length".to_string()),
        },
        Err(e) => Err(e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::chart_ax::{Axis, DataPoint, Series};

    #[test]
    fn tones_travel_as_pitch_and_length() {
        let j = tones_json(&[Tone { hz: 440.0, millis: 250, timbre: 0 }, Tone { hz: 0.0, millis: 0, timbre: 1 }]);
        let Some(Json::Arr(list)) = j.get("tones") else { panic!("no tones") };
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].get("hz"), Some(&Json::Num(440.0)));
        assert_eq!(list[0].get("ms"), Some(&Json::Num(250.0)));
        assert_eq!(list[1].get("ms"), Some(&Json::Num(1.0)), "a zero length is sent as the shortest the shell takes");
    }

    #[test]
    fn a_chart_series_becomes_its_audio_graph() {
        let points = vec![
            DataPoint { label: "Jan".into(), y: 0.0, extra: vec![] },
            DataPoint { label: "Feb".into(), y: f64::NAN, extra: vec![] },
            DataPoint { label: "Mar".into(), y: 10.0, extra: vec![] },
        ];
        let chart = ChartDescriptor::new(
            "Rain",
            "",
            Axis::Categorical { title: "Month".into(), order: vec![] },
            Some(Axis::Numeric { title: "mm".into(), range: (0.0, 10.0), unit: "mm".into(), decimals: 0 }),
            vec![Series { name: "2026".into(), is_continuous: false, points }],
        );
        let tones = chart.audio_graph(0);
        let j = tones_json(&tones);
        let Some(Json::Arr(list)) = j.get("tones") else { panic!() };
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].get("hz"), Some(&Json::Num(0.0)), "a missing value is a rest");
        assert!(chart.audio_graph(5).is_empty(), "an unknown series has no graph, so play sends nothing");
    }

    #[test]
    fn the_reply_is_the_length() {
        assert_eq!(length_from_reply(&Ok(Json::Obj(vec![("ms".into(), Json::Num(750.0))]))), Ok(750));
        assert!(length_from_reply(&Ok(Json::Null)).is_err());
        assert_eq!(length_from_reply(&Err("only the app in front".into())), Err("only the app in front".into()));
    }
}
