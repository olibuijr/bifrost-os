//! `lastcomm`: lists the commands recorded by process accounting, newest first. The system writes one 64-byte record per
//! finished process to the accounting file (version 3 of the record layout). A record holds the command name, the
//! process flags, the user id, the start time (seconds since the epoch), the elapsed time and the user and system CPU
//! time. CPU times are stored in the compressed form of the layout (13 bits of mantissa, 3 bits of exponent) counted in
//! clock ticks; `TICKS_PER_SECOND` is the rate the system uses for them.
//!
//! The version byte carries a byte-order flag: when it is set, the numbers of the record are big-endian.
//!
//! Gap: the tool prints user ids, not user names, and it does not read rotated accounting files.

pub const USAGE: &str = "usage: lastcomm [command ...]";
/// The file the system writes process accounting records to.
pub const ACCT_FILE: &str = "/var/account/pacct";
/// The size of one version 3 record.
pub const RECORD_LEN: usize = 64;
/// Clock ticks per second in the CPU time fields.
pub const TICKS_PER_SECOND: f64 = 100.0;

const VERSION: u8 = 3;
const BIG_ENDIAN_FLAG: u8 = 0x80;
const FLAG_FORKED: u8 = 0x01;
const FLAG_SUPERUSER: u8 = 0x02;
const FLAG_CORE_DUMPED: u8 = 0x08;
const FLAG_SIGNALLED: u8 = 0x10;

/// What the command line asks for: the usage text, or a listing filtered by command names (empty for all).
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Vec<String>),
}

/// One finished process.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub command: String,
    pub superuser: bool,
    pub forked: bool,
    pub core_dumped: bool,
    pub signalled: bool,
    pub uid: u32,
    /// Seconds since the epoch when the process started.
    pub start: u32,
    /// Seconds from start to exit.
    pub elapsed: f32,
    pub user_seconds: f64,
    pub system_seconds: f64,
}

/// Reads the arguments: `-h` and `--help` ask for the usage text, every other argument is a command name.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(Parsed::Help);
    }
    if let Some(bad) = args.iter().find(|a| a.starts_with('-')) {
        return Err(format!("unknown option {bad}"));
    }
    Ok(Parsed::Run(args.to_vec()))
}

/// The value of a compressed count of clock ticks: the mantissa shifted left by three bits per exponent step.
pub fn comp_t(value: u16) -> f64 {
    let mantissa = f64::from(value & 0x1fff);
    mantissa * f64::from(1u32 << (3 * u32::from(value >> 13)))
}

/// Decodes the records of an accounting file, oldest first.
pub fn parse_records(bytes: &[u8]) -> Result<Vec<Record>, String> {
    if bytes.len() % RECORD_LEN != 0 {
        return Err(format!("accounting data is {} bytes, not a whole number of {RECORD_LEN}-byte records", bytes.len()));
    }
    bytes.chunks_exact(RECORD_LEN).map(record).collect()
}

fn record(raw: &[u8]) -> Result<Record, String> {
    let version = raw[1];
    if version & !BIG_ENDIAN_FLAG != VERSION {
        return Err(format!("unsupported accounting record version {}", version & !BIG_ENDIAN_FLAG));
    }
    let big_endian = version & BIG_ENDIAN_FLAG != 0;
    let word = |at: usize| -> [u8; 4] { [raw[at], raw[at + 1], raw[at + 2], raw[at + 3]] };
    let u32_at = |at: usize| if big_endian { u32::from_be_bytes(word(at)) } else { u32::from_le_bytes(word(at)) };
    let u16_at = |at: usize| {
        let bytes = [raw[at], raw[at + 1]];
        if big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        }
    };
    let flag = raw[0];
    let name_bytes = &raw[48..64];
    let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
    let elapsed = f32::from_bits(u32_at(28));
    if !elapsed.is_finite() || elapsed < 0.0 {
        return Err("accounting record has an invalid elapsed time".to_string());
    }
    Ok(Record {
        command: String::from_utf8_lossy(&name_bytes[..name_end]).into_owned(),
        superuser: flag & FLAG_SUPERUSER != 0,
        forked: flag & FLAG_FORKED != 0,
        core_dumped: flag & FLAG_CORE_DUMPED != 0,
        signalled: flag & FLAG_SIGNALLED != 0,
        uid: u32_at(8),
        start: u32_at(24),
        elapsed,
        user_seconds: comp_t(u16_at(32)) / TICKS_PER_SECOND,
        system_seconds: comp_t(u16_at(34)) / TICKS_PER_SECOND,
    })
}

/// The flag letters of a record: `S` superuser, `F` forked without exec, `D` core dumped, `X` signalled.
pub fn flags(record: &Record) -> String {
    [(record.superuser, 'S'), (record.forked, 'F'), (record.core_dumped, 'D'), (record.signalled, 'X')]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, c)| *c)
        .collect()
}

/// One line per record, newest first, keeping only the commands in `names` when it is not empty.
pub fn render(records: &[Record], names: &[String]) -> String {
    records
        .iter()
        .rev()
        .filter(|r| names.is_empty() || names.iter().any(|n| *n == r.command))
        .map(|r| {
            format!(
                "{:<16} {:<4} uid {:<6} start {:>10} {:>9.2} secs (user {:.2}, system {:.2})\n",
                r.command,
                flags(r),
                r.uid,
                r.start,
                r.elapsed,
                r.user_seconds,
                r.system_seconds
            )
        })
        .collect()
}

/// The listing of an accounting file's bytes, filtered by command names.
pub fn report(bytes: &[u8], names: &[String]) -> Result<String, String> {
    parse_records(bytes).map(|records| render(&records, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct_v3(big_endian: bool, command: &[u8]) -> Vec<u8> {
        let mut raw = vec![0; RECORD_LEN];
        raw[0] = FLAG_SUPERUSER | FLAG_FORKED | FLAG_CORE_DUMPED | FLAG_SIGNALLED;
        raw[1] = VERSION | if big_endian { BIG_ENDIAN_FLAG } else { 0 };
        let put_u16 = |raw: &mut [u8], at: usize, value: u16| {
            let bytes = if big_endian { value.to_be_bytes() } else { value.to_le_bytes() };
            raw[at..at + 2].copy_from_slice(&bytes);
        };
        let put_u32 = |raw: &mut [u8], at: usize, value: u32| {
            let bytes = if big_endian { value.to_be_bytes() } else { value.to_le_bytes() };
            raw[at..at + 4].copy_from_slice(&bytes);
        };
        put_u32(&mut raw, 8, 1001);
        put_u32(&mut raw, 24, 1_700_000_000);
        put_u32(&mut raw, 28, 1.25f32.to_bits());
        put_u16(&mut raw, 32, 10_000);
        put_u16(&mut raw, 34, 2_000);
        let len = command.len().min(16);
        raw[48..48 + len].copy_from_slice(&command[..len]);
        raw
    }

    #[test]
    fn parses_linux_acct_v3_fields_and_flags() {
        let record = parse_records(&acct_v3(false, b"shell")).unwrap().remove(0);
        assert_eq!(record.command, "shell");
        assert_eq!(record.uid, 1001);
        assert_eq!(record.start, 1_700_000_000);
        assert_eq!(record.elapsed, 1.25);
        assert_eq!(record.user_seconds, 100.0);
        assert_eq!(record.system_seconds, 20.0);
        assert_eq!(flags(&record), "SFDX");
    }

    #[test]
    fn honours_the_big_endian_version_flag() {
        let record = parse_records(&acct_v3(true, b"worker")).unwrap().remove(0);
        assert_eq!(record.uid, 1001);
        assert_eq!(record.start, 1_700_000_000);
        assert_eq!(record.command, "worker");
    }

    #[test]
    fn compressed_cpu_times_expand_the_exponent() {
        assert_eq!(comp_t(0x2001), 8.0);
        assert_eq!(comp_t(0x4001), 64.0);
    }

    #[test]
    fn a_partial_record_and_an_unknown_version_are_refused() {
        assert!(parse_records(&[0; RECORD_LEN - 1]).unwrap_err().contains("whole number"));
        let mut bad = acct_v3(false, b"shell");
        bad[1] = 4;
        assert!(parse_records(&bad).unwrap_err().contains("unsupported accounting record version 4"));
    }

    #[test]
    fn invalid_elapsed_times_are_refused() {
        let mut bad = acct_v3(false, b"shell");
        bad[28..32].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert!(parse_records(&bad).unwrap_err().contains("invalid elapsed time"));
    }

    #[test]
    fn filters_commands_and_renders_newest_first() {
        let mut bytes = acct_v3(false, b"old");
        bytes.extend_from_slice(&acct_v3(false, b"new"));
        let all = report(&bytes, &[]).unwrap();
        assert!(all.find("new").unwrap() < all.find("old").unwrap());
        assert_eq!(render(&parse_records(&bytes).unwrap(), &["old".into()]).matches('\n').count(), 1);
    }

    #[test]
    fn help_and_unknown_options_are_distinguished() {
        assert_eq!(parse(&["--help".into()]), Ok(Parsed::Help));
        assert_eq!(parse(&["-z".into()]), Err("unknown option -z".into()));
        assert_eq!(parse(&["shell".into()]), Ok(Parsed::Run(vec!["shell".into()])));
    }
}
