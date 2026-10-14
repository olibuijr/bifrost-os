//! Host Controller Interface packets: the 16-bit opcode of a command (a 6-bit group and a 10-bit command within it),
//! command packets with their parameters, and the event packets a controller answers with, including the command
//! complete event. The byte layout follows the Bluetooth core specification (volume 2, part E). Pure: the shell talks
//! to the controller through the management API and does not send these packets itself.

/// Why a packet could not be built or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HciError {
    /// The group is 64 or more, or the command is 1024 or more, so the opcode does not fit.
    OpcodeOutOfRange,
    /// The parameters are longer than the 255 bytes a command header can count.
    ParametersTooLong,
    /// The bytes end before the length the header declares.
    Truncated,
}

/// A command opcode: the group (OGF, bits 10 to 15) and the command (OCF, bits 0 to 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Opcode(u16);

impl Opcode {
    /// The opcode of command `ocf` in group `ogf`. Refused when either field is too wide.
    pub fn new(ogf: u8, ocf: u16) -> Result<Opcode, HciError> {
        if ogf >= 64 || ocf >= 1024 {
            return Err(HciError::OpcodeOutOfRange);
        }
        Ok(Opcode(u16::from(ogf) << 10 | ocf))
    }

    /// The raw 16-bit value.
    pub fn raw(self) -> u16 {
        self.0
    }

    /// The command group.
    pub fn ogf(self) -> u8 {
        (self.0 >> 10) as u8
    }

    /// The command within its group.
    pub fn ocf(self) -> u16 {
        self.0 & 0x03FF
    }
}

/// Inquiry: scan for nearby devices (link control group).
pub const OP_INQUIRY: (u8, u16) = (0x01, 0x0001);
/// Disconnect: end a link (link control group).
pub const OP_DISCONNECT: (u8, u16) = (0x01, 0x0006);
/// Remote Name Request: ask a device for its name (link control group).
pub const OP_REMOTE_NAME_REQUEST: (u8, u16) = (0x01, 0x0019);
/// Reset the controller (controller and baseband group).
pub const OP_RESET: (u8, u16) = (0x03, 0x0003);
/// Read the controller's device address (informational group).
pub const OP_READ_BD_ADDR: (u8, u16) = (0x04, 0x0009);
/// LE Set Scan Enable (LE controller group).
pub const OP_LE_SET_SCAN_ENABLE: (u8, u16) = (0x08, 0x000C);

/// Event codes of the events this module reads.
pub const EVT_INQUIRY_COMPLETE: u8 = 0x01;
pub const EVT_DISCONNECTION_COMPLETE: u8 = 0x05;
pub const EVT_COMMAND_COMPLETE: u8 = 0x0E;
pub const EVT_COMMAND_STATUS: u8 = 0x0F;
pub const EVT_LE_META: u8 = 0x3E;

/// A command packet ready to send: its opcode and parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub opcode: Opcode,
    pub parameters: Vec<u8>,
}

impl Command {
    /// A command with `parameters`. Refused when they are longer than a command header can count.
    pub fn new(group: (u8, u16), parameters: Vec<u8>) -> Result<Command, HciError> {
        if parameters.len() > usize::from(u8::MAX) {
            return Err(HciError::ParametersTooLong);
        }
        Ok(Command { opcode: Opcode::new(group.0, group.1)?, parameters })
    }

    /// The bytes on the wire: the opcode (little endian), the parameter length, then the parameters.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(3 + self.parameters.len());
        out.extend(self.opcode.raw().to_le_bytes());
        out.push(self.parameters.len() as u8);
        out.extend(&self.parameters);
        out
    }
}

/// An event packet: its code and its parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub code: u8,
    pub parameters: Vec<u8>,
}

impl Event {
    /// Reads one event packet from the start of `bytes`. The declared parameter length must fit in what is there.
    pub fn parse(bytes: &[u8]) -> Result<Event, HciError> {
        let (&code, rest) = bytes.split_first().ok_or(HciError::Truncated)?;
        let (&len, rest) = rest.split_first().ok_or(HciError::Truncated)?;
        let parameters = rest.get(..usize::from(len)).ok_or(HciError::Truncated)?;
        Ok(Event { code, parameters: parameters.to_vec() })
    }
}

/// The status byte a controller reports. Only the statuses the shell can meet are named; the rest read as
/// [`HciStatus::Other`] with their code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HciStatus {
    Success,
    UnknownCommand,
    UnknownConnectionIdentifier,
    AuthenticationFailure,
    ConnectionTimeout,
    RemoteUserTerminated,
    ConnectionTerminatedByLocalHost,
    Other(u8),
}

impl HciStatus {
    pub fn from_code(code: u8) -> HciStatus {
        match code {
            0x00 => HciStatus::Success,
            0x01 => HciStatus::UnknownCommand,
            0x02 => HciStatus::UnknownConnectionIdentifier,
            0x05 => HciStatus::AuthenticationFailure,
            0x08 => HciStatus::ConnectionTimeout,
            0x13 => HciStatus::RemoteUserTerminated,
            0x16 => HciStatus::ConnectionTerminatedByLocalHost,
            other => HciStatus::Other(other),
        }
    }
}

/// A command complete event: how many command packets the controller can take, which command finished, its status
/// and the return parameters that follow the status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandComplete {
    pub command_packets: u8,
    pub opcode: Opcode,
    pub status: HciStatus,
    pub return_parameters: Vec<u8>,
}

impl CommandComplete {
    /// Reads the parameters of a command complete event (code [`EVT_COMMAND_COMPLETE`]).
    pub fn parse(event: &Event) -> Result<CommandComplete, HciError> {
        if event.code != EVT_COMMAND_COMPLETE {
            return Err(HciError::Truncated);
        }
        let p = &event.parameters;
        if p.len() < 4 {
            return Err(HciError::Truncated);
        }
        let opcode = Opcode(u16::from_le_bytes([p[1], p[2]]));
        Ok(CommandComplete {
            command_packets: p[0],
            opcode,
            status: HciStatus::from_code(p[3]),
            return_parameters: p[4..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opcode_packs_the_group_into_the_top_six_bits() {
        assert_eq!(Opcode::new(0x01, 0x0001).unwrap().raw(), 0x0401);
        assert_eq!(Opcode::new(0x03, 0x0003).unwrap().raw(), 0x0C03);
        assert_eq!(Opcode::new(0x08, 0x000C).unwrap().raw(), 0x200C);
    }

    #[test]
    fn an_opcode_reads_back_its_group_and_command() {
        let op = Opcode::new(0x01, 0x0019).unwrap();
        assert_eq!(op.ogf(), 0x01);
        assert_eq!(op.ocf(), 0x0019);
    }

    #[test]
    fn an_opcode_field_too_wide_is_refused() {
        assert_eq!(Opcode::new(64, 0), Err(HciError::OpcodeOutOfRange));
        assert_eq!(Opcode::new(0, 1024), Err(HciError::OpcodeOutOfRange));
    }

    #[test]
    fn a_command_encodes_its_opcode_little_endian_then_its_length_and_parameters() {
        let cmd = Command::new(OP_DISCONNECT, vec![0x40, 0x00, 0x13]).unwrap();
        assert_eq!(cmd.encode(), vec![0x06, 0x04, 0x03, 0x40, 0x00, 0x13]);
    }

    #[test]
    fn a_command_without_parameters_encodes_a_zero_length() {
        let cmd = Command::new(OP_RESET, Vec::new()).unwrap();
        assert_eq!(cmd.encode(), vec![0x03, 0x0C, 0x00]);
    }

    #[test]
    fn a_command_with_too_many_parameters_is_refused() {
        assert_eq!(Command::new(OP_RESET, vec![0; 256]), Err(HciError::ParametersTooLong));
        assert!(Command::new(OP_RESET, vec![0; 255]).is_ok());
    }

    #[test]
    fn an_event_reads_its_code_and_declared_parameters() {
        let event = Event::parse(&[0x05, 0x02, 0xAA, 0xBB, 0xCC]).unwrap();
        assert_eq!(event.code, EVT_DISCONNECTION_COMPLETE);
        assert_eq!(event.parameters, vec![0xAA, 0xBB]);
    }

    #[test]
    fn an_event_whose_length_runs_past_the_bytes_is_truncated() {
        assert_eq!(Event::parse(&[0x05, 0x04, 0xAA]), Err(HciError::Truncated));
        assert_eq!(Event::parse(&[0x05]), Err(HciError::Truncated));
        assert_eq!(Event::parse(&[]), Err(HciError::Truncated));
    }

    #[test]
    fn a_command_complete_event_names_the_command_and_its_status() {
        // One command packet is free; the reset command finished with status 0x00 and no return parameters.
        let event = Event::parse(&[EVT_COMMAND_COMPLETE, 0x04, 0x01, 0x03, 0x0C, 0x00]).unwrap();
        let done = CommandComplete::parse(&event).unwrap();
        assert_eq!(done.command_packets, 1);
        assert_eq!(done.opcode, Opcode::new(0x03, 0x0003).unwrap());
        assert_eq!(done.status, HciStatus::Success);
        assert!(done.return_parameters.is_empty());
    }

    #[test]
    fn a_command_complete_event_keeps_its_return_parameters() {
        let event = Event::parse(&[EVT_COMMAND_COMPLETE, 0x0A, 0x01, 0x09, 0x10, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]).unwrap();
        let done = CommandComplete::parse(&event).unwrap();
        assert_eq!(done.opcode.raw(), 0x1009);
        assert_eq!(done.status, HciStatus::Success);
        assert_eq!(done.return_parameters.len(), 6);
    }

    #[test]
    fn other_event_codes_are_not_command_complete() {
        let event = Event { code: EVT_INQUIRY_COMPLETE, parameters: vec![0, 0, 0, 0] };
        assert_eq!(CommandComplete::parse(&event), Err(HciError::Truncated));
    }

    #[test]
    fn named_statuses_map_and_unknown_codes_keep_their_value() {
        assert_eq!(HciStatus::from_code(0x05), HciStatus::AuthenticationFailure);
        assert_eq!(HciStatus::from_code(0x08), HciStatus::ConnectionTimeout);
        assert_eq!(HciStatus::from_code(0x3A), HciStatus::Other(0x3A));
    }
}
