//! Process objects (`kAudioHardwarePropertyProcessObjectList`) and process taps (`CATapDescription`,
//! `AudioHardwareCreateProcessTap`), as pure models with their wire shapes. Not implemented: the audiod side. The
//! service does not yet report processes or hook output, so the replies decoded here come from a fake in tests.
use bifrost_core::json::Json;

/// `kAudioHardwarePropertyProcessObjectList` ('prs#').
pub const PROPERTY_PROCESS_OBJECT_LIST: u32 = u32::from_be_bytes(*b"prs#");
/// `kAudioHardwarePropertyTapList` ('tps#').
pub const PROPERTY_TAP_LIST: u32 = u32::from_be_bytes(*b"tps#");

/// An audio process object: `kAudioProcessPropertyPID`, `...BundleID`, `...IsRunningInput`, `...IsRunningOutput`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessObject {
    pub pid: u32,
    pub bundle_id: String,
    pub running_input: bool,
    pub running_output: bool,
}

impl ProcessObject {
    /// `kAudioProcessPropertyIsRunning`: doing input or output.
    pub fn is_running(&self) -> bool {
        self.running_input || self.running_output
    }
}

/// The request body that lists the process objects.
pub fn list_request() -> Json {
    Json::Obj(vec![("property".to_string(), Json::Num(PROPERTY_PROCESS_OBJECT_LIST as f64))])
}

/// Reads a process list reply: `{"processes":[{"pid","bundle_id","running_input","running_output"}]}`.
pub fn decode_processes(v: &Json) -> Result<Vec<ProcessObject>, String> {
    let items = v.get("processes").and_then(|p| p.as_array()).ok_or_else(|| "the reply has no processes".to_string())?;
    items
        .iter()
        .map(|p| {
            let pid = p.get("pid").and_then(|x| x.number()).ok_or_else(|| "a process has no pid".to_string())?;
            let flag = |k: &str| matches!(p.get(k), Some(Json::Bool(true)));
            Ok(ProcessObject {
                pid: pid as u32,
                bundle_id: p.str_field("bundle_id").unwrap_or_default(),
                running_input: flag("running_input"),
                running_output: flag("running_output"),
            })
        })
        .collect()
}

/// What happens to the tapped audio on the speakers (`CATapMuteBehavior`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteBehavior {
    Unmuted,
    Muted,
    MutedWhenTapped,
}

impl MuteBehavior {
    fn name(self) -> &'static str {
        match self {
            MuteBehavior::Unmuted => "unmuted",
            MuteBehavior::Muted => "muted",
            MuteBehavior::MutedWhenTapped => "muted_when_tapped",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "unmuted" => MuteBehavior::Unmuted,
            "muted" => MuteBehavior::Muted,
            "muted_when_tapped" => MuteBehavior::MutedWhenTapped,
            _ => return None,
        })
    }
}

/// `CATapDescription`. `processes` are the tapped pids, or the excluded ones when `exclusive`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapDescription {
    pub name: String,
    pub processes: Vec<u32>,
    pub mono: bool,
    pub exclusive: bool,
    pub mixdown: bool,
    pub mute_behavior: MuteBehavior,
    pub private: bool,
}

impl TapDescription {
    fn base(processes: Vec<u32>, mono: bool, exclusive: bool) -> Self {
        TapDescription {
            name: String::new(),
            processes,
            mono,
            exclusive,
            mixdown: true,
            mute_behavior: MuteBehavior::Unmuted,
            private: false,
        }
    }

    /// `init(stereoMixdownOfProcesses:)`
    pub fn stereo_mixdown_of(processes: Vec<u32>) -> Self {
        Self::base(processes, false, false)
    }

    /// `init(monoMixdownOfProcesses:)`
    pub fn mono_mixdown_of(processes: Vec<u32>) -> Self {
        Self::base(processes, true, false)
    }

    /// `init(stereoGlobalTapButExcludeProcesses:)`
    pub fn stereo_global_excluding(processes: Vec<u32>) -> Self {
        Self::base(processes, false, true)
    }

    /// `init(monoGlobalTapButExcludeProcesses:)`
    pub fn mono_global_excluding(processes: Vec<u32>) -> Self {
        Self::base(processes, true, true)
    }

    /// A tap of named processes needs at least one; a global tap may exclude none.
    pub fn validate(&self) -> Result<(), String> {
        if !self.exclusive && self.processes.is_empty() {
            return Err("a tap of specific processes needs at least one process".into());
        }
        Ok(())
    }

    /// Whether the tap captures the audio of `pid`.
    pub fn captures(&self, pid: u32) -> bool {
        self.processes.contains(&pid) != self.exclusive
    }

    /// The channel count the tap delivers.
    pub fn channels(&self) -> u32 {
        if self.mono {
            1
        } else {
            2
        }
    }

    /// The request body that creates the tap.
    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("name".to_string(), Json::Str(self.name.clone())),
            ("processes".to_string(), Json::Arr(self.processes.iter().map(|p| Json::Num(*p as f64)).collect())),
            ("mono".to_string(), Json::Bool(self.mono)),
            ("exclusive".to_string(), Json::Bool(self.exclusive)),
            ("mixdown".to_string(), Json::Bool(self.mixdown)),
            ("mute".to_string(), Json::Str(self.mute_behavior.name().to_string())),
            ("private".to_string(), Json::Bool(self.private)),
        ])
    }

    /// Reads a request body back; the result is validated.
    pub fn from_json(v: &Json) -> Result<Self, String> {
        let processes = v
            .get("processes")
            .and_then(|p| p.as_array())
            .ok_or_else(|| "no processes".to_string())?
            .iter()
            .filter_map(|p| p.number().map(|n| n as u32))
            .collect();
        let flag = |k: &str| matches!(v.get(k), Some(Json::Bool(true)));
        let mute_behavior = match v.str_field("mute") {
            Some(m) => MuteBehavior::parse(&m).ok_or_else(|| format!("unknown mute behavior {m}"))?,
            None => MuteBehavior::Unmuted,
        };
        let t = TapDescription {
            name: v.str_field("name").unwrap_or_default(),
            processes,
            mono: flag("mono"),
            exclusive: flag("exclusive"),
            mixdown: flag("mixdown"),
            mute_behavior,
            private: flag("private"),
        };
        t.validate()?;
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Json {
        bifrost_core::json::parse(text).unwrap()
    }

    #[test]
    fn a_process_list_reply_decodes() {
        let v = parsed(
            r#"{"processes":[{"pid":10,"bundle_id":"is.olibuijr.music","running_output":true},
                {"pid":11,"bundle_id":"is.olibuijr.memos","running_input":true}]}"#,
        );
        let list = decode_processes(&v).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].running_output && !list[0].running_input && list[0].is_running());
        assert!(list[1].running_input);
        assert!(decode_processes(&parsed(r#"{}"#)).is_err());
        assert!(decode_processes(&parsed(r#"{"processes":[{"bundle_id":"x"}]}"#)).is_err());
        let idle = ProcessObject { pid: 1, bundle_id: String::new(), running_input: false, running_output: false };
        assert!(!idle.is_running());
    }

    #[test]
    fn property_selectors_are_four_char_codes() {
        assert_eq!(PROPERTY_PROCESS_OBJECT_LIST, 0x7072_7323);
        assert_eq!(PROPERTY_TAP_LIST, 0x7470_7323);
        assert_eq!(list_request().get("property").and_then(|p| p.number()), Some(PROPERTY_PROCESS_OBJECT_LIST as f64));
    }

    #[test]
    fn constructors_set_mono_and_exclusive() {
        assert_eq!(TapDescription::stereo_mixdown_of(vec![1]).channels(), 2);
        assert_eq!(TapDescription::mono_mixdown_of(vec![1]).channels(), 1);
        assert!(TapDescription::stereo_global_excluding(vec![]).exclusive);
        assert!(TapDescription::mono_global_excluding(vec![]).mono);
    }

    #[test]
    fn capture_follows_inclusion_or_exclusion() {
        let t = TapDescription::stereo_mixdown_of(vec![5]);
        assert!(t.captures(5) && !t.captures(6));
        let g = TapDescription::stereo_global_excluding(vec![5]);
        assert!(!g.captures(5) && g.captures(6));
    }

    #[test]
    fn a_tap_of_no_processes_is_refused_but_a_global_one_is_fine() {
        assert!(TapDescription::stereo_mixdown_of(vec![]).validate().is_err());
        assert!(TapDescription::stereo_global_excluding(vec![]).validate().is_ok());
    }

    #[test]
    fn a_tap_round_trips_with_its_mute_behavior() {
        let mut t = TapDescription::mono_mixdown_of(vec![3, 4]);
        t.name = "call".into();
        t.mute_behavior = MuteBehavior::MutedWhenTapped;
        assert_eq!(TapDescription::from_json(&t.to_json()).unwrap(), t);
        let bad = parsed(r#"{"processes":[1],"mute":"loud"}"#);
        assert!(TapDescription::from_json(&bad).is_err());
    }
}
