//! Hands-free profile session model: the AT-command dialogue between an audio gateway and a hands-free device,
//! the indicator table, the feature masks, the service-level connection steps and the framing of command lines
//! read from a byte stream. The radio, the RFCOMM transport and the SCO audio link belong to the system; this module
//! only decides what a line means and what a session does with it. Command lines end with CR; replies may end with
//! CR LF.

/// Feature bits an audio gateway reports in its `+BRSF` reply.
pub mod ag_feature {
    pub const THREE_WAY: u32 = 1 << 0;
    pub const EC_NR: u32 = 1 << 1;
    pub const VOICE_RECOGNITION: u32 = 1 << 2;
    pub const IN_BAND_RING: u32 = 1 << 3;
    pub const VOICE_TAG: u32 = 1 << 4;
    pub const REJECT_CALL: u32 = 1 << 5;
    pub const ENHANCED_STATUS: u32 = 1 << 6;
    pub const ENHANCED_CONTROL: u32 = 1 << 7;
    pub const EXTENDED_ERRORS: u32 = 1 << 8;
    pub const CODEC_NEGOTIATION: u32 = 1 << 9;
}

/// Feature bits a hands-free device sends in its `AT+BRSF` command.
pub mod hf_feature {
    pub const EC_NR: u32 = 1 << 0;
    pub const CALL_WAITING: u32 = 1 << 1;
    pub const CALLER_ID: u32 = 1 << 2;
    pub const VOICE_RECOGNITION: u32 = 1 << 3;
    pub const REMOTE_VOLUME: u32 = 1 << 4;
    pub const ENHANCED_STATUS: u32 = 1 << 5;
    pub const ENHANCED_CONTROL: u32 = 1 << 6;
    pub const CODEC_NEGOTIATION: u32 = 1 << 7;
    pub const HF_INDICATORS: u32 = 1 << 8;
    pub const ESCO_S4: u32 = 1 << 9;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HfpError {
    /// A value is outside the range its indicator allows.
    OutOfRange,
    /// A reply or indicator list has the wrong shape.
    Malformed,
    /// The gateway answered `ERROR` during the step that was running.
    Rejected(Step),
}

/// The indicators of the gateway, in the order the `+CIND` list gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    Service,
    Call,
    CallSetup,
    CallHeld,
    Signal,
    Roam,
    Battery,
}

impl Indicator {
    pub const ALL: [Indicator; 7] = [
        Indicator::Service,
        Indicator::Call,
        Indicator::CallSetup,
        Indicator::CallHeld,
        Indicator::Signal,
        Indicator::Roam,
        Indicator::Battery,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Indicator::Service => "service",
            Indicator::Call => "call",
            Indicator::CallSetup => "callsetup",
            Indicator::CallHeld => "callheld",
            Indicator::Signal => "signal",
            Indicator::Roam => "roam",
            Indicator::Battery => "battery",
        }
    }

    /// Inclusive minimum and maximum of the indicator.
    pub fn range(self) -> (i32, i32) {
        match self {
            Indicator::Service => (0, 1),
            Indicator::Call => (0, 1),
            Indicator::CallSetup => (0, 3),
            Indicator::CallHeld => (0, 2),
            Indicator::Signal => (0, 5),
            Indicator::Roam => (0, 1),
            Indicator::Battery => (0, 5),
        }
    }

    fn index(self) -> usize {
        match self {
            Indicator::Service => 0,
            Indicator::Call => 1,
            Indicator::CallSetup => 2,
            Indicator::CallHeld => 3,
            Indicator::Signal => 4,
            Indicator::Roam => 5,
            Indicator::Battery => 6,
        }
    }

    /// The 1-based position used by `+CIEV` updates.
    pub fn position(self) -> usize {
        self.index() + 1
    }
}

/// The current value of every indicator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Indicators {
    values: [i32; 7],
}

impl Indicators {
    pub fn get(&self, ind: Indicator) -> i32 {
        self.values[ind.index()]
    }

    /// Stores a value and reports whether it changed. A value outside the indicator's range is refused.
    pub fn set(&mut self, ind: Indicator, value: i32) -> Result<bool, HfpError> {
        let (lo, hi) = ind.range();
        if value < lo || value > hi {
            return Err(HfpError::OutOfRange);
        }
        let slot = &mut self.values[ind.index()];
        let changed = *slot != value;
        *slot = value;
        Ok(changed)
    }

    /// The reply to `AT+CIND=?`: each indicator with its name and range.
    pub fn list_reply() -> String {
        let items: Vec<String> = Indicator::ALL
            .iter()
            .map(|ind| {
                let (lo, hi) = ind.range();
                format!("(\"{}\",({}-{}))", ind.name(), lo, hi)
            })
            .collect();
        format!("+CIND: {}", items.join(","))
    }

    /// The reply to `AT+CIND?`: the current values in list order.
    pub fn status_reply(&self) -> String {
        let values: Vec<String> = self.values.iter().map(|v| v.to_string()).collect();
        format!("+CIND: {}", values.join(","))
    }
}

/// A command a hands-free device sends to the gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `AT+BRSF=<features>`
    Brsf(u32),
    /// `AT+CIND=?`
    CindTest,
    /// `AT+CIND?`
    CindRead,
    /// `AT+CMER=3,0,0,<enable>`: turns indicator reporting on or off.
    Cmer { enable: bool },
    /// `ATA`
    Answer,
    /// `AT+CHUP`
    HangUp,
    /// `ATD<number>;`: a voice dial. Memory dials (`ATD>`) are not modelled.
    Dial(String),
    /// `AT+VGS=<0-15>`: speaker volume.
    Speaker(u8),
    /// `AT+VGM=<0-15>`: microphone gain.
    Mic(u8),
    /// Any line this model does not handle, kept as text.
    Unknown(String),
}

fn volume(arg: &str) -> Option<u8> {
    match arg.parse::<u8>() {
        Ok(v) if v <= 15 => Some(v),
        _ => None,
    }
}

/// Reads one command line. Keywords are case-insensitive. Returns `None` for an empty line.
pub fn parse_command(line: &str) -> Option<Command> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    let upper = text.to_ascii_uppercase();
    let body = match upper.strip_prefix("AT") {
        Some(body) => body,
        None => return Some(Command::Unknown(text.to_string())),
    };
    let command = if body == "A" {
        Command::Answer
    } else if body == "+CHUP" {
        Command::HangUp
    } else if body == "+CIND=?" {
        Command::CindTest
    } else if body == "+CIND?" {
        Command::CindRead
    } else if let Some(arg) = body.strip_prefix("+BRSF=") {
        match arg.parse::<u32>() {
            Ok(v) => Command::Brsf(v),
            Err(_) => Command::Unknown(text.to_string()),
        }
    } else if let Some(arg) = body.strip_prefix("+CMER=") {
        let fields: Vec<&str> = arg.split(',').collect();
        if fields.len() == 4 {
            Command::Cmer { enable: fields[3] == "1" }
        } else {
            Command::Unknown(text.to_string())
        }
    } else if let Some(arg) = body.strip_prefix("+VGS=") {
        match volume(arg) {
            Some(v) => Command::Speaker(v),
            None => Command::Unknown(text.to_string()),
        }
    } else if let Some(arg) = body.strip_prefix("+VGM=") {
        match volume(arg) {
            Some(v) => Command::Mic(v),
            None => Command::Unknown(text.to_string()),
        }
    } else if let Some(number) = body.strip_prefix('D').and_then(|rest| rest.strip_suffix(';')) {
        if number.is_empty() {
            Command::Unknown(text.to_string())
        } else {
            Command::Dial(number.to_string())
        }
    } else {
        Command::Unknown(text.to_string())
    };
    Some(command)
}

/// A reply or unsolicited line the gateway sends to the hands-free device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Error,
    Ring,
    /// `+BRSF: <features>`
    Brsf(u32),
    /// `+CIND: <value>,...` with one value per indicator.
    Values(Vec<i32>),
    /// `+CIEV: <position>,<value>`
    Ciev(usize, i32),
    /// Anything else, kept as text. This includes the `+CIND: (...)` list.
    Other(String),
}

/// Reads one reply line.
pub fn parse_reply(line: &str) -> Reply {
    let text = line.trim();
    match text {
        "OK" => return Reply::Ok,
        "ERROR" => return Reply::Error,
        "RING" => return Reply::Ring,
        _ => {}
    }
    if let Some(rest) = text.strip_prefix("+BRSF:") {
        return match rest.trim().parse::<u32>() {
            Ok(v) => Reply::Brsf(v),
            Err(_) => Reply::Other(text.to_string()),
        };
    }
    if let Some(rest) = text.strip_prefix("+CIEV:") {
        let mut parts = rest.split(',');
        let index = parts.next().and_then(|p| p.trim().parse::<usize>().ok());
        let value = parts.next().and_then(|p| p.trim().parse::<i32>().ok());
        return match (index, value, parts.next()) {
            (Some(i), Some(v), None) => Reply::Ciev(i, v),
            _ => Reply::Other(text.to_string()),
        };
    }
    if let Some(rest) = text.strip_prefix("+CIND:") {
        if !rest.trim_start().starts_with('(') {
            let parsed: Option<Vec<i32>> = rest.split(',').map(|p| p.trim().parse::<i32>().ok()).collect();
            if let Some(values) = parsed {
                return Reply::Values(values);
            }
        }
    }
    Reply::Other(text.to_string())
}

/// What a gateway does when a device's command has been read and applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Answer,
    HangUp,
    Dial(String),
    Speaker(u8),
    Mic(u8),
}

/// The gateway side of one service-level connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gateway {
    ag_features: u32,
    hf_features: u32,
    indicators: Indicators,
    reporting: bool,
    speaker: u8,
    mic: u8,
}

fn ok() -> String {
    "OK".to_string()
}

impl Gateway {
    pub fn new(ag_features: u32) -> Self {
        Self { ag_features, ..Self::default() }
    }

    pub fn hf_features(&self) -> u32 {
        self.hf_features
    }

    pub fn reporting(&self) -> bool {
        self.reporting
    }

    pub fn indicators(&self) -> &Indicators {
        &self.indicators
    }

    pub fn speaker(&self) -> u8 {
        self.speaker
    }

    pub fn mic(&self) -> u8 {
        self.mic
    }

    /// Applies a command. Returns the reply lines to send and, for a call or volume action, the event for the shell.
    pub fn handle(&mut self, command: Command) -> (Vec<String>, Option<Event>) {
        match command {
            Command::Brsf(features) => {
                self.hf_features = features;
                (vec![format!("+BRSF: {}", self.ag_features), ok()], None)
            }
            Command::CindTest => (vec![Indicators::list_reply(), ok()], None),
            Command::CindRead => (vec![self.indicators.status_reply(), ok()], None),
            Command::Cmer { enable } => {
                self.reporting = enable;
                (vec![ok()], None)
            }
            Command::Answer => (vec![ok()], Some(Event::Answer)),
            Command::HangUp => (vec![ok()], Some(Event::HangUp)),
            Command::Dial(number) => (vec![ok()], Some(Event::Dial(number))),
            Command::Speaker(v) => {
                self.speaker = v;
                (vec![ok()], Some(Event::Speaker(v)))
            }
            Command::Mic(v) => {
                self.mic = v;
                (vec![ok()], Some(Event::Mic(v)))
            }
            Command::Unknown(_) => (vec!["ERROR".to_string()], None),
        }
    }

    /// Sets an indicator. Returns the `+CIEV` line to send when reporting is on and the value changed.
    pub fn set_indicator(&mut self, ind: Indicator, value: i32) -> Result<Option<String>, HfpError> {
        let changed = self.indicators.set(ind, value)?;
        if changed && self.reporting {
            Ok(Some(format!("+CIEV: {},{}", ind.position(), value)))
        } else {
            Ok(None)
        }
    }
}

/// The steps of the service-level connection a hands-free device runs after the link is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    Features,
    IndicatorList,
    IndicatorStatus,
    Reporting,
    Connected,
    Failed,
}

fn with_cr(body: &str) -> String {
    format!("{}\r", body)
}

/// The hands-free device side of one service-level connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandsFree {
    hf_features: u32,
    ag_features: u32,
    step: Step,
    indicators: Indicators,
}

impl HandsFree {
    pub fn new(hf_features: u32) -> Self {
        Self { hf_features, ag_features: 0, step: Step::Idle, indicators: Indicators::default() }
    }

    pub fn step(&self) -> Step {
        self.step
    }

    pub fn ag_features(&self) -> u32 {
        self.ag_features
    }

    pub fn indicators(&self) -> &Indicators {
        &self.indicators
    }

    /// Starts the connection from `Idle` or after a failure. Returns the first command line to send.
    pub fn connect(&mut self) -> Option<String> {
        if !matches!(self.step, Step::Idle | Step::Failed) {
            return None;
        }
        self.step = Step::Features;
        Some(with_cr(&format!("AT+BRSF={}", self.hf_features)))
    }

    /// Reads one reply. Returns the next command line when the step is finished.
    pub fn receive(&mut self, reply: &Reply) -> Result<Option<String>, HfpError> {
        match reply {
            Reply::Error => {
                let from = self.step;
                self.step = Step::Failed;
                Err(HfpError::Rejected(from))
            }
            Reply::Ok => Ok(self.advance()),
            Reply::Brsf(features) => {
                if self.step == Step::Features {
                    self.ag_features = *features;
                }
                Ok(None)
            }
            Reply::Values(values) => {
                if self.step == Step::IndicatorStatus {
                    self.apply_values(values)?;
                }
                Ok(None)
            }
            Reply::Ciev(index, value) => {
                if self.step == Step::Connected {
                    let position = (*index)
                        .checked_sub(1)
                        .and_then(|i| Indicator::ALL.get(i).copied())
                        .ok_or(HfpError::OutOfRange)?;
                    self.indicators.set(position, *value)?;
                }
                Ok(None)
            }
            Reply::Ring | Reply::Other(_) => Ok(None),
        }
    }

    fn apply_values(&mut self, values: &[i32]) -> Result<(), HfpError> {
        if values.len() != Indicator::ALL.len() {
            return Err(HfpError::Malformed);
        }
        for (ind, value) in Indicator::ALL.iter().zip(values.iter()) {
            self.indicators.set(*ind, *value)?;
        }
        Ok(())
    }

    fn advance(&mut self) -> Option<String> {
        let (next, out) = match self.step {
            Step::Features => (Step::IndicatorList, Some("AT+CIND=?")),
            Step::IndicatorList => (Step::IndicatorStatus, Some("AT+CIND?")),
            Step::IndicatorStatus => (Step::Reporting, Some("AT+CMER=3,0,0,1")),
            Step::Reporting => (Step::Connected, None),
            _ => return None,
        };
        self.step = next;
        out.map(with_cr)
    }
}

/// Splits a byte stream into lines. A line ends at CR or LF; empty lines are dropped and the unfinished tail waits
/// for the next push.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Framer {
    pending: String,
}

impl Framer {
    pub fn push(&mut self, bytes: &str) -> Vec<String> {
        self.pending.push_str(bytes);
        let mut lines = Vec::new();
        while let Some(pos) = self.pending.find(|c: char| c == '\r' || c == '\n') {
            let raw: String = self.pending.drain(..=pos).collect();
            let text = raw.trim();
            if !text.is_empty() {
                lines.push(text.to_string());
            }
        }
        lines
    }

    pub fn pending(&self) -> &str {
        &self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn command_keywords_are_case_insensitive_and_parse_features() {
        assert_eq!(parse_command("AT+BRSF=1023"), Some(Command::Brsf(1023)));
        assert_eq!(parse_command("at+cind=?"), Some(Command::CindTest));
        assert_eq!(parse_command("AT+CIND?"), Some(Command::CindRead));
        assert_eq!(parse_command("ATA"), Some(Command::Answer));
        assert_eq!(parse_command("AT+CHUP"), Some(Command::HangUp));
        assert_eq!(parse_command("   "), None);
    }

    #[test]
    fn reporting_flag_is_the_fourth_cmer_field() {
        assert_eq!(parse_command("AT+CMER=3,0,0,1"), Some(Command::Cmer { enable: true }));
        assert_eq!(parse_command("AT+CMER=3,0,0,0"), Some(Command::Cmer { enable: false }));
        assert_eq!(parse_command("AT+CMER=3,0,0"), Some(Command::Unknown("AT+CMER=3,0,0".to_string())));
    }

    #[test]
    fn dial_needs_the_trailing_semicolon() {
        assert_eq!(parse_command("ATD5551234;"), Some(Command::Dial("5551234".to_string())));
        assert_eq!(parse_command("ATD5551234"), Some(Command::Unknown("ATD5551234".to_string())));
        assert_eq!(parse_command("ATD;"), Some(Command::Unknown("ATD;".to_string())));
    }

    #[test]
    fn volume_is_limited_to_zero_through_fifteen() {
        assert_eq!(parse_command("AT+VGS=15"), Some(Command::Speaker(15)));
        assert_eq!(parse_command("AT+VGM=0"), Some(Command::Mic(0)));
        assert_eq!(parse_command("AT+VGS=16"), Some(Command::Unknown("AT+VGS=16".to_string())));
    }

    #[test]
    fn indicator_table_has_the_list_and_status_text_in_order() {
        let indicators = Indicators::default();
        assert_eq!(indicators.status_reply(), "+CIND: 0,0,0,0,0,0,0");
        assert_eq!(
            Indicators::list_reply(),
            "+CIND: (\"service\",(0-1)),(\"call\",(0-1)),(\"callsetup\",(0-3)),(\"callheld\",(0-2)),(\"signal\",(0-5)),(\"roam\",(0-1)),(\"battery\",(0-5))"
        );
        assert_eq!(Indicator::Call.position(), 2);
        assert_eq!(Indicator::Battery.position(), 7);
    }

    #[test]
    fn indicator_set_reports_change_and_refuses_out_of_range() {
        let mut indicators = Indicators::default();
        assert_eq!(indicators.set(Indicator::Call, 1), Ok(true));
        assert_eq!(indicators.set(Indicator::Call, 1), Ok(false));
        assert_eq!(indicators.set(Indicator::Signal, 6), Err(HfpError::OutOfRange));
        assert_eq!(indicators.get(Indicator::Signal), 0);
    }

    #[test]
    fn gateway_answers_features_and_reports_indicator_changes() {
        let mut gw = Gateway::new(ag_feature::VOICE_RECOGNITION | ag_feature::ENHANCED_STATUS);
        let (lines, event) = gw.handle(Command::Brsf(0x3F));
        assert_eq!(lines, strs(&["+BRSF: 68", "OK"]));
        assert_eq!(event, None);
        assert_eq!(gw.hf_features(), 0x3F);

        assert_eq!(gw.handle(Command::Cmer { enable: true }).0, strs(&["OK"]));
        assert_eq!(gw.set_indicator(Indicator::Call, 1), Ok(Some("+CIEV: 2,1".to_string())));
        assert_eq!(gw.set_indicator(Indicator::Call, 1), Ok(None));
        assert_eq!(gw.set_indicator(Indicator::Signal, 6), Err(HfpError::OutOfRange));
    }

    #[test]
    fn gateway_emits_shell_events_and_refuses_unknown_commands() {
        let mut gw = Gateway::new(0);
        assert_eq!(gw.handle(Command::Speaker(7)), (strs(&["OK"]), Some(Event::Speaker(7))));
        assert_eq!(gw.speaker(), 7);
        assert_eq!(gw.handle(Command::Dial("5551234".to_string())), (strs(&["OK"]), Some(Event::Dial("5551234".to_string()))));
        assert_eq!(gw.handle(Command::Unknown("AT+XYZ".to_string())), (strs(&["ERROR"]), None));
    }

    #[test]
    fn hands_free_runs_the_connection_steps_in_order() {
        let mut link = HandsFree::new(3);
        assert_eq!(link.connect(), Some("AT+BRSF=3\r".to_string()));
        assert_eq!(link.receive(&parse_reply("+BRSF: 68")), Ok(None));
        assert_eq!(link.receive(&parse_reply("OK")), Ok(Some("AT+CIND=?\r".to_string())));
        assert_eq!(link.receive(&parse_reply("+CIND: (\"service\",(0-1))")), Ok(None));
        assert_eq!(link.receive(&Reply::Ok), Ok(Some("AT+CIND?\r".to_string())));
        assert_eq!(link.receive(&parse_reply("+CIND: 1,0,0,0,5,0,4")), Ok(None));
        assert_eq!(link.receive(&Reply::Ok), Ok(Some("AT+CMER=3,0,0,1\r".to_string())));
        assert_eq!(link.receive(&Reply::Ok), Ok(None));
        assert_eq!(link.step(), Step::Connected);
        assert_eq!(link.ag_features(), 68);
        assert_eq!(link.indicators().get(Indicator::Service), 1);
        assert_eq!(link.indicators().get(Indicator::Signal), 5);
        assert_eq!(link.indicators().get(Indicator::Battery), 4);

        assert_eq!(link.receive(&parse_reply("+CIEV: 2,1")), Ok(None));
        assert_eq!(link.indicators().get(Indicator::Call), 1);
    }

    #[test]
    fn hands_free_records_a_rejection_and_can_reconnect() {
        let mut link = HandsFree::new(0);
        assert_eq!(link.connect(), Some("AT+BRSF=0\r".to_string()));
        assert_eq!(link.receive(&Reply::Error), Err(HfpError::Rejected(Step::Features)));
        assert_eq!(link.step(), Step::Failed);
        assert_eq!(link.connect(), Some("AT+BRSF=0\r".to_string()));
    }

    #[test]
    fn values_reply_with_the_wrong_count_is_malformed() {
        let mut link = HandsFree::new(0);
        link.connect();
        link.receive(&Reply::Ok).unwrap();
        link.receive(&Reply::Ok).unwrap();
        assert_eq!(link.step(), Step::IndicatorStatus);
        assert_eq!(link.receive(&parse_reply("+CIND: 1,0,0")), Err(HfpError::Malformed));
        assert_eq!(link.indicators().get(Indicator::Service), 0);
        assert_eq!(link.receive(&Reply::Ok), Ok(Some("AT+CMER=3,0,0,1\r".to_string())));
    }

    #[test]
    fn framer_splits_on_cr_and_lf_and_keeps_the_tail() {
        let mut framer = Framer::default();
        assert_eq!(framer.push("AT+BRSF=3"), Vec::<String>::new());
        assert_eq!(framer.pending(), "AT+BRSF=3");
        assert_eq!(framer.push("\r"), strs(&["AT+BRSF=3"]));
        assert_eq!(framer.push("OK\r\n+BRSF: 68\r"), strs(&["OK", "+BRSF: 68"]));
        assert_eq!(framer.pending(), "");
    }
}
