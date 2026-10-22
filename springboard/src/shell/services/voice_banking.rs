//! The `voice_banking` service: the sample store for Voice Banking, the personal synthetic voice made for accessibility
//! speech. The user reads each script phrase; the app records it and reports the sample here. The shell keeps the
//! metadata of the samples (which phrase, the sample rate, the frame count) in the `a11y_voice_bank` pref, so the
//! progress survives a restart. The audio itself stays with the app, and the shell does not train a voice from it. The
//! app measures each recording with `bifrost_ui::voice_profile::analyze` and may send the result as `profile`; the shell
//! keeps those measurements (pref `a11y_voice_profiles`) and `voice()` merges them into the speaker's pitch and pace,
//! which `VoiceProfile::apply` puts on the rule-based synthesiser. A trained voice (timbre) is not built.
//!
//! Methods: `phrases()` returns the script as an array of strings; `progress()` returns `{"recorded", "total", "ready"}`
//! where `ready` is true once every phrase has a good sample; `record({"phrase", "sample_rate", "frames"})` stores one
//! sample for a phrase, replacing an earlier one for the same phrase, and returns the new progress. A sample is good
//! when its rate is 8 kHz to 48 kHz and it lasts 0.5 s to 15 s. `record` also takes an optional `profile` (the text form
//! of a `VoiceProfile`); a malformed one refuses the whole call. `voice()` returns `null` until a profile is stored, then
//! `{"samples", "mean_f0_hz", "low_f0_hz", "high_f0_hz", "syllables_per_second", "pitch", "rate"}` where `pitch` and
//! `rate` are the synthesiser settings.
use super::{Shell, SystemService};
use crate::shell::prefs::Prefs;
use bifrost_core::json::Json;
use bifrost_ui::voice_profile::VoiceProfile;

/// The service name apps call it by.
pub const SERVICE: &str = "voice_banking";
/// The method that returns the script.
pub const METHOD_PHRASES: &str = "phrases";
/// The method that returns the progress.
pub const METHOD_PROGRESS: &str = "progress";
/// The method that stores one sample.
pub const METHOD_RECORD: &str = "record";
/// The method that returns the speaker's merged voice settings.
pub const METHOD_VOICE: &str = "voice";
/// The pref that holds the bank.
pub const PREF: &str = "a11y_voice_bank";
/// The pref that holds the measured profile of each sample, `phrase=profile;...`.
pub const PROFILES_PREF: &str = "a11y_voice_profiles";
/// The phrases the user reads, one each.
pub const PHRASES: [&str; 4] = ["The quick brown fox jumps over the lazy dog.", "Please read the next line aloud.", "Open my messages and read them to me.", "Turn the page when I say next."];
/// The sample rates a sample may have.
pub const RATE_RANGE: (u32, u32) = (8_000, 48_000);
/// The lengths a sample may have, in seconds: 0.5 s to 15 s at its own rate.
pub const SECONDS_RANGE: (f64, f64) = (0.5, 15.0);

/// One stored sample: the phrase it reads, the sample rate and the number of frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub phrase: usize,
    pub sample_rate: u32,
    pub frames: u32,
}

impl Sample {
    /// Whether the rate and the length are in the accepted ranges.
    pub fn is_good(&self) -> bool {
        let seconds = self.frames as f64 / self.sample_rate.max(1) as f64;
        (RATE_RANGE.0..=RATE_RANGE.1).contains(&self.sample_rate) && (SECONDS_RANGE.0..=SECONDS_RANGE.1).contains(&seconds)
    }
}

/// The samples read so far, at most one per phrase.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bank {
    samples: Vec<Sample>,
    profiles: Vec<(usize, VoiceProfile)>,
}

impl Bank {
    /// Parses the pref `phrase:rate:frames;...`. Entries that are malformed, name no phrase or are not good are dropped.
    pub fn parse(s: &str) -> Bank {
        let mut bank = Bank::default();
        for entry in s.split(';') {
            let parts: Vec<&str> = entry.split(':').collect();
            let [phrase, rate, frames] = parts.as_slice() else { continue };
            let (Ok(phrase), Ok(sample_rate), Ok(frames)) = (phrase.parse::<usize>(), rate.parse::<u32>(), frames.parse::<u32>()) else { continue };
            let sample = Sample { phrase, sample_rate, frames };
            if phrase < PHRASES.len() && sample.is_good() {
                bank.put(sample);
            }
        }
        bank
    }

    /// The pref text, samples in phrase order.
    pub fn encode(&self) -> String {
        self.samples.iter().map(|s| format!("{}:{}:{}", s.phrase, s.sample_rate, s.frames)).collect::<Vec<_>>().join(";")
    }

    /// Reads the bank from the prefs, with the profiles of the samples it holds.
    pub fn load(p: &Prefs) -> Bank {
        let mut bank = Bank::parse(p.get(PREF).unwrap_or(""));
        bank.parse_profiles(p.get(PROFILES_PREF).unwrap_or(""));
        bank
    }

    /// Reads `phrase=profile;...`. Entries that are malformed or whose phrase has no sample are dropped.
    pub fn parse_profiles(&mut self, s: &str) {
        for entry in s.split(';') {
            let Some((phrase, text)) = entry.split_once('=') else { continue };
            let (Ok(phrase), Ok(profile)) = (phrase.parse::<usize>(), VoiceProfile::parse(text)) else { continue };
            if self.samples.iter().any(|x| x.phrase == phrase) {
                self.put_profile(phrase, profile);
            }
        }
    }

    /// The profile pref text, in phrase order.
    pub fn encode_profiles(&self) -> String {
        self.profiles.iter().map(|(phrase, p)| format!("{phrase}={}", p.encode())).collect::<Vec<_>>().join(";")
    }

    fn put_profile(&mut self, phrase: usize, profile: VoiceProfile) {
        self.profiles.retain(|(x, _)| *x != phrase);
        self.profiles.push((phrase, profile));
        self.profiles.sort_by_key(|(x, _)| *x);
    }

    /// Keeps the measured `profile` of the sample for `phrase`. Refuses a phrase with no stored sample.
    pub fn record_profile(&mut self, phrase: usize, profile: VoiceProfile) -> Result<(), String> {
        if !self.samples.iter().any(|s| s.phrase == phrase) {
            return Err(format!("phrase {phrase} has no sample to attach a profile to"));
        }
        self.put_profile(phrase, profile);
        Ok(())
    }

    /// The speaker's voice: every stored profile merged, weighted by voiced frames. `None` before any profile.
    pub fn voice(&self) -> Option<VoiceProfile> {
        let all: Vec<VoiceProfile> = self.profiles.iter().map(|(_, p)| *p).collect();
        VoiceProfile::merge(&all)
    }

    /// Stores `sample` in place of any earlier one for its phrase. Refuses a phrase that is not in the script or a sample
    /// that is not good, and says why.
    pub fn record(&mut self, sample: Sample) -> Result<(), String> {
        if sample.phrase >= PHRASES.len() {
            return Err(format!("no phrase {}", sample.phrase));
        }
        if !sample.is_good() {
            return Err("the sample is too short, too long or has an unsupported rate".to_string());
        }
        self.put(sample);
        Ok(())
    }

    fn put(&mut self, sample: Sample) {
        self.samples.retain(|s| s.phrase != sample.phrase);
        // a new recording makes the old measurement stale
        self.profiles.retain(|(x, _)| *x != sample.phrase);
        self.samples.push(sample);
        self.samples.sort_by_key(|s| s.phrase);
    }

    /// How many phrases have a good sample.
    pub fn recorded(&self) -> usize {
        self.samples.len()
    }

    /// Whether every phrase in the script has a good sample.
    pub fn ready(&self) -> bool {
        self.recorded() == PHRASES.len()
    }
}

/// Answers one service call against `bank`. `record` changes the bank; the caller saves it.
pub fn answer(bank: &mut Bank, method: &str, params: &Json) -> Result<Json, String> {
    match method {
        METHOD_PHRASES => Ok(Json::Arr(PHRASES.iter().map(|p| Json::Str(p.to_string())).collect())),
        METHOD_PROGRESS => Ok(progress_json(bank)),
        METHOD_RECORD => {
            let num = |k: &str| params.get(k).and_then(|v| if let Json::Num(n) = v { Some(*n) } else { None });
            let (Some(phrase), Some(rate), Some(frames)) = (num("phrase"), num("sample_rate"), num("frames")) else {
                return Err("record needs phrase, sample_rate and frames".to_string());
            };
            if phrase < 0.0 || rate < 0.0 || frames < 0.0 || phrase.fract() != 0.0 || rate.fract() != 0.0 || frames.fract() != 0.0 {
                return Err("record takes whole, non-negative numbers".to_string());
            }
            let profile = match params.get("profile") {
                None | Some(Json::Null) => None,
                Some(Json::Str(text)) => Some(VoiceProfile::parse(text).map_err(|_| "profile is not a voice profile".to_string())?),
                Some(_) => return Err("profile must be text".to_string()),
            };
            bank.record(Sample { phrase: phrase as usize, sample_rate: rate as u32, frames: frames as u32 })?;
            if let Some(profile) = profile {
                bank.record_profile(phrase as usize, profile)?;
            }
            Ok(progress_json(bank))
        }
        METHOD_VOICE => Ok(match bank.voice() {
            None => Json::Null,
            Some(v) => Json::Obj(vec![
                ("samples".to_string(), Json::Num(bank.profiles.len() as f64)),
                ("mean_f0_hz".to_string(), Json::Num(v.mean_f0_hz as f64)),
                ("low_f0_hz".to_string(), Json::Num(v.low_f0_hz as f64)),
                ("high_f0_hz".to_string(), Json::Num(v.high_f0_hz as f64)),
                ("syllables_per_second".to_string(), Json::Num(v.syllables_per_second as f64)),
                ("pitch".to_string(), Json::Num(v.pitch_multiplier(bifrost_ui::voice_profile::REFERENCE_F0_HZ) as f64)),
                ("rate".to_string(), Json::Num(v.rate_setting() as f64)),
            ]),
        }),
        _ => Err(format!("voice_banking has no method {method}")),
    }
}

fn progress_json(bank: &Bank) -> Json {
    Json::Obj(vec![
        ("recorded".to_string(), Json::Num(bank.recorded() as f64)),
        ("total".to_string(), Json::Num(PHRASES.len() as f64)),
        ("ready".to_string(), Json::Bool(bank.ready())),
    ])
}

pub struct VoiceBanking;

impl SystemService for VoiceBanking {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, _app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        let mut bank = Bank::load(&shell.prefs);
        let reply = answer(&mut bank, method, params)?;
        if method == METHOD_RECORD {
            let text = bank.encode();
            let profiles = bank.encode_profiles();
            shell.prefs.set_many(&[(PREF, text.as_str()), (PROFILES_PREF, profiles.as_str())]).map_err(|e| format!("could not save the voice bank: {e}"))?;
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(phrase: f64, rate: f64, frames: f64) -> Json {
        Json::Obj(vec![("phrase".into(), Json::Num(phrase)), ("sample_rate".into(), Json::Num(rate)), ("frames".into(), Json::Num(frames))])
    }

    #[test]
    fn a_fresh_bank_has_nothing_recorded() {
        let b = Bank::default();
        assert_eq!(b.recorded(), 0);
        assert!(!b.ready());
        assert_eq!(answer(&mut Bank::default(), METHOD_PROGRESS, &Json::Null).unwrap().get("recorded"), Some(&Json::Num(0.0)));
    }

    #[test]
    fn a_good_sample_is_kept_and_a_second_one_replaces_it() {
        let mut b = Bank::default();
        b.record(Sample { phrase: 1, sample_rate: 16_000, frames: 40_000 }).unwrap();
        b.record(Sample { phrase: 1, sample_rate: 48_000, frames: 96_000 }).unwrap();
        assert_eq!(b.recorded(), 1);
        assert_eq!(b.encode(), "1:48000:96000");
    }

    #[test]
    fn bad_samples_are_refused_with_a_reason() {
        let mut b = Bank::default();
        assert!(b.record(Sample { phrase: PHRASES.len(), sample_rate: 16_000, frames: 40_000 }).is_err());
        assert!(b.record(Sample { phrase: 0, sample_rate: 4_000, frames: 40_000 }).is_err(), "rate too low");
        assert!(b.record(Sample { phrase: 0, sample_rate: 16_000, frames: 1_000 }).is_err(), "shorter than 0.5 s");
        assert!(b.record(Sample { phrase: 0, sample_rate: 16_000, frames: 16_000 * 16 }).is_err(), "longer than 15 s");
        assert_eq!(b.recorded(), 0);
    }

    #[test]
    fn the_bank_is_ready_once_every_phrase_has_a_sample() {
        let mut b = Bank::default();
        for phrase in 0..PHRASES.len() {
            assert!(!b.ready());
            b.record(Sample { phrase, sample_rate: 22_050, frames: 22_050 }).unwrap();
        }
        assert!(b.ready());
        assert_eq!(b.recorded(), PHRASES.len());
    }

    #[test]
    fn the_pref_round_trips_and_damaged_entries_are_dropped() {
        let b = Bank::parse("2:16000:32000;x:1:1;0:16000;9:16000:32000;1:16000:100");
        assert_eq!(b.encode(), "2:16000:32000");
        assert_eq!(Bank::parse(&b.encode()), b);
    }

    #[test]
    fn record_over_the_service_answers_with_progress() {
        let mut b = Bank::default();
        let reply = answer(&mut b, METHOD_RECORD, &rec(3.0, 16_000.0, 32_000.0)).unwrap();
        assert_eq!(reply.get("recorded"), Some(&Json::Num(1.0)));
        assert_eq!(reply.get("total"), Some(&Json::Num(PHRASES.len() as f64)));
        assert!(answer(&mut b, METHOD_RECORD, &rec(0.5, 16_000.0, 32_000.0)).is_err(), "a fractional phrase is refused");
        assert!(answer(&mut b, METHOD_RECORD, &Json::Null).is_err());
    }

    #[test]
    fn phrases_returns_the_whole_script() {
        match answer(&mut Bank::default(), METHOD_PHRASES, &Json::Null).unwrap() {
            Json::Arr(items) => assert_eq!(items.len(), PHRASES.len()),
            other => panic!("expected an array, got {other:?}"),
        }
    }

    #[test]
    fn the_service_is_named_as_the_sdk_expects() {
        assert_eq!(VoiceBanking.name(), "voice_banking");
        assert_eq!(SERVICE, "voice_banking");
    }

    fn rec_with(phrase: f64, profile: Json) -> Json {
        Json::Obj(vec![
            ("phrase".into(), Json::Num(phrase)),
            ("sample_rate".into(), Json::Num(16_000.0)),
            ("frames".into(), Json::Num(32_000.0)),
            ("profile".into(), profile),
        ])
    }

    #[test]
    fn a_profile_sent_with_a_sample_is_kept_and_merged_into_the_voice() {
        let mut b = Bank::default();
        assert_eq!(answer(&mut b, METHOD_VOICE, &Json::Null).unwrap(), Json::Null);
        answer(&mut b, METHOD_RECORD, &rec_with(0.0, Json::Str("100.00:90.00:110.00:3.000:0.10000:100".into()))).unwrap();
        answer(&mut b, METHOD_RECORD, &rec_with(1.0, Json::Str("200.00:190.00:210.00:5.000:0.10000:300".into()))).unwrap();
        let voice = answer(&mut b, METHOD_VOICE, &Json::Null).unwrap();
        assert_eq!(voice.get("samples"), Some(&Json::Num(2.0)));
        let Some(Json::Num(mean)) = voice.get("mean_f0_hz") else { panic!("no mean") };
        assert!((mean - 175.0).abs() < 0.01, "mean {mean}");
        let Some(Json::Num(pitch)) = voice.get("pitch") else { panic!("no pitch") };
        assert!((pitch - 1.25).abs() < 0.001, "pitch {pitch}");
    }

    #[test]
    fn a_malformed_profile_refuses_the_whole_record() {
        let mut b = Bank::default();
        assert!(answer(&mut b, METHOD_RECORD, &rec_with(0.0, Json::Str("loud".into()))).is_err());
        assert!(answer(&mut b, METHOD_RECORD, &rec_with(0.0, Json::Num(3.0))).is_err());
        assert_eq!(b.recorded(), 0, "nothing was stored");
    }

    #[test]
    fn a_new_recording_drops_the_old_measurement() {
        let mut b = Bank::default();
        answer(&mut b, METHOD_RECORD, &rec_with(2.0, Json::Str("120.00:110.00:130.00:4.000:0.10000:50".into()))).unwrap();
        assert!(b.voice().is_some());
        answer(&mut b, METHOD_RECORD, &rec(2.0, 16_000.0, 32_000.0)).unwrap();
        assert!(b.voice().is_none());
        assert_eq!(b.encode_profiles(), "");
    }

    #[test]
    fn a_profile_needs_a_stored_sample() {
        let mut b = Bank::default();
        let p = VoiceProfile::parse("120.00:110.00:130.00:4.000:0.10000:50").unwrap();
        assert!(b.record_profile(1, p).is_err());
    }

    #[test]
    fn the_profile_pref_round_trips_and_drops_orphans() {
        let mut b = Bank::parse("1:16000:32000");
        b.parse_profiles("1=120.00:110.00:130.00:4.000:0.10000:50;3=120.00:110.00:130.00:4.000:0.10000:50;x=1;1");
        assert_eq!(b.encode_profiles(), "1=120.00:110.00:130.00:4.000:0.10000:50");
        let mut again = Bank::parse(&b.encode());
        again.parse_profiles(&b.encode_profiles());
        assert_eq!(again, b);
    }
}
