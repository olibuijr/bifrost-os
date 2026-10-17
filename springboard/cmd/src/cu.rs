//! `cu`: opens a serial line as an interactive terminal. The reference tool's `-l line` and `-s speed` (or `-speed`)
//! options are read, and the session is handed to `picocom`, which the picocom package provides. The speed defaults to
//! 9600 baud. Gap: dialing a telephone number is refused (there is no modem dialer here), and the reference tool's
//! escape sequences are picocom's own (Ctrl-A then Ctrl-X quits).

pub const USAGE: &str = "usage: cu -l line [-s speed] [-h]";
pub const DEFAULT_SPEED: &str = "9600";

/// The program that opens the line. It comes from the picocom package.
pub const PROGRAM: &str = "picocom";

/// What the command line asks for: the help text, or a line and its speed.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Open { line: String, speed: String },
}

/// Reads the arguments. A telephone number (a bare operand) is refused, and so is a speed that is not a number.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut line: Option<String> = None;
    let mut speed: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-l" => {
                let value = it.next().ok_or("-l needs a serial line, such as /dev/ttyUSB0")?;
                line = Some(value.clone());
            }
            "-s" => {
                let value = it.next().ok_or("-s needs a speed")?;
                speed = Some(checked_speed(value)?);
            }
            other if other.starts_with('-') && other.len() > 1 => {
                // `-115200` is the reference tool's spelling of `-s 115200`.
                let digits = &other[1..];
                if digits.chars().all(|c| c.is_ascii_digit()) {
                    speed = Some(checked_speed(digits)?);
                } else {
                    return Err(format!("option {other} is not supported yet\n{USAGE}"));
                }
            }
            _ => return Err("dialing a telephone number is not supported; use -l with a serial line".into()),
        }
    }
    let line = line.ok_or_else(|| USAGE.to_string())?;
    Ok(Parsed::Open { line, speed: speed.unwrap_or_else(|| DEFAULT_SPEED.to_string()) })
}

fn checked_speed(value: &str) -> Result<String, String> {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
        Ok(value.to_string())
    } else {
        Err(format!("speed must be a number of baud, not {value}"))
    }
}

/// The argument list for `picocom`: the baud rate, then the serial line.
pub fn picocom_args(line: &str, speed: &str) -> Vec<String> {
    vec!["-b".to_string(), speed.to_string(), line.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_line_opens_at_nine_six_hundred_by_default() {
        assert_eq!(parse(&args(&["-l", "/dev/ttyUSB0"])), Ok(Parsed::Open { line: "/dev/ttyUSB0".into(), speed: "9600".into() }));
    }

    #[test]
    fn speed_is_read_in_both_spellings() {
        assert_eq!(parse(&args(&["-s", "115200", "-l", "/dev/ttyS0"])), Ok(Parsed::Open { line: "/dev/ttyS0".into(), speed: "115200".into() }));
        assert_eq!(parse(&args(&["-19200", "-l", "/dev/ttyS1"])), Ok(Parsed::Open { line: "/dev/ttyS1".into(), speed: "19200".into() }));
    }

    #[test]
    fn telephone_numbers_and_bad_speeds_are_refused() {
        assert!(parse(&args(&["555-1234"])).unwrap_err().contains("telephone"));
        assert!(parse(&args(&["-s", "fast", "-l", "/dev/ttyS0"])).unwrap_err().contains("speed must be a number"));
        assert_eq!(parse(&args(&[])), Err(USAGE.to_string()));
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
    }

    #[test]
    fn picocom_gets_the_baud_rate_then_the_line() {
        assert_eq!(picocom_args("/dev/ttyS0", "115200"), args(&["-b", "115200", "/dev/ttyS0"]));
    }
}
