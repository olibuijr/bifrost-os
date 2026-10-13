//! The connection handshake: protocol version, security choice, the security result and ServerInit.
//!
//! The viewer answers with the highest version both sides know (3.8, 3.7 or 3.3). Only security None is accepted;
//! a server that offers only VNC authentication gets `Error::Unsupported`.

use crate::{Error, PixelFormat};
use std::io::{Read, Write};

/// The protocol version the server announced, as major and minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
}

/// Security type None: no authentication.
pub const SECURITY_NONE: u8 = 1;
/// Security type VNC Authentication (DES challenge). Not implemented by this viewer.
pub const SECURITY_VNC: u8 = 2;

/// ServerInit: the framebuffer size, the server's pixel format and the desktop name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInit {
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
    pub name: String,
}

/// Parses a 12-byte ProtocolVersion string such as `RFB 003.008\n`.
pub fn parse_version(b: &[u8]) -> Result<Version, Error> {
    if b.len() != 12 || &b[..4] != b"RFB " || b[7] != b'.' || b[11] != b'\n' {
        return Err(Error::Protocol("not an RFB server".into()));
    }
    let num = |s: &[u8]| -> Result<u8, Error> {
        std::str::from_utf8(s).ok().and_then(|t| t.parse::<u16>().ok()).filter(|n| *n <= 255).map(|n| n as u8).ok_or_else(|| Error::Protocol("bad version number".into()))
    };
    Ok(Version { major: num(&b[4..7])?, minor: num(&b[8..11])? })
}

/// The version string this viewer answers with for a server at `server`: 3.8 and 3.7 are echoed, anything older is
/// 3.3 (the only other version the viewer speaks), and a newer server is answered with 3.8.
pub fn answer_version(server: Version) -> Result<Version, Error> {
    if server.major != 3 {
        return Err(Error::Unsupported(format!("RFB {}.{}", server.major, server.minor)));
    }
    Ok(match server.minor {
        m if m >= 8 => Version { major: 3, minor: 8 },
        7 => Version { major: 3, minor: 7 },
        _ => Version { major: 3, minor: 3 },
    })
}

/// The 12-byte version string for `v`.
pub fn version_bytes(v: Version) -> [u8; 12] {
    let s = format!("RFB {:03}.{:03}\n", v.major, v.minor);
    let mut out = [0u8; 12];
    out.copy_from_slice(s.as_bytes());
    out
}

/// Reads the server's version, answers with the viewer's version, and returns the agreed version.
pub fn negotiate_version<S: Read + Write>(s: &mut S) -> Result<Version, Error> {
    let mut b = [0u8; 12];
    s.read_exact(&mut b)?;
    let server = parse_version(&b)?;
    let answer = answer_version(server)?;
    s.write_all(&version_bytes(answer))?;
    Ok(answer)
}

/// Reads the security types the server offers and picks None. In 3.3 the server chooses for us, so the reply is
/// the one type it sends; in 3.7 and later the viewer answers with the type it picked.
pub fn negotiate_security<S: Read + Write>(s: &mut S, v: Version) -> Result<(), Error> {
    if v.minor == 3 {
        return match read_u32(s)? {
            0 => Err(Error::Protocol(read_reason(s)?)),
            1 => Ok(()),
            2 => Err(Error::Unsupported("password (VNC) authentication".into())),
            t => Err(Error::Unsupported(format!("security type {t}"))),
        };
    }
    let mut count = [0u8; 1];
    s.read_exact(&mut count)?;
    if count[0] == 0 {
        return Err(Error::Protocol(read_reason(s)?));
    }
    let mut types = vec![0u8; count[0] as usize];
    s.read_exact(&mut types)?;
    if !types.contains(&SECURITY_NONE) {
        return if types.contains(&SECURITY_VNC) {
            Err(Error::Unsupported("password (VNC) authentication".into()))
        } else {
            Err(Error::Unsupported(format!("security types {types:?}")))
        };
    }
    s.write_all(&[SECURITY_NONE])?;
    Ok(())
}

/// Reads the SecurityResult that follows the security choice. It is sent for every type in 3.8; in 3.3 and 3.7 it
/// is not sent for None, so this returns at once for those versions.
pub fn read_security_result<S: Read>(s: &mut S, v: Version) -> Result<(), Error> {
    if v.minor < 8 {
        return Ok(());
    }
    match read_u32(s)? {
        0 => Ok(()),
        _ => Err(Error::Protocol(read_reason(s)?)),
    }
}

/// The ClientInit message: shared = true, so other viewers of this desktop stay connected.
pub fn client_init_bytes(shared: bool) -> [u8; 1] {
    [shared as u8]
}

/// Reads ServerInit.
pub fn read_server_init<S: Read>(s: &mut S) -> Result<ServerInit, Error> {
    let mut head = [0u8; 24];
    s.read_exact(&mut head)?;
    let width = u16::from_be_bytes([head[0], head[1]]);
    let height = u16::from_be_bytes([head[2], head[3]]);
    let format = PixelFormat::parse(&head[4..20])?;
    let len = u32::from_be_bytes([head[20], head[21], head[22], head[23]]) as usize;
    if len > 4096 {
        return Err(Error::Protocol("desktop name too long".into()));
    }
    let mut name = vec![0u8; len];
    s.read_exact(&mut name)?;
    Ok(ServerInit { width, height, format, name: String::from_utf8_lossy(&name).into_owned() })
}

/// Runs the whole handshake up to ServerInit.
pub fn connect<S: Read + Write>(s: &mut S) -> Result<ServerInit, Error> {
    let v = negotiate_version(s)?;
    negotiate_security(s, v)?;
    read_security_result(s, v)?;
    s.write_all(&client_init_bytes(true))?;
    read_server_init(s)
}

fn read_u32<S: Read>(s: &mut S) -> Result<u32, Error> {
    let mut b = [0u8; 4];
    s.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

/// A failure reason: a length-prefixed string the server sends before it closes the connection.
fn read_reason<S: Read>(s: &mut S) -> Result<String, Error> {
    let len = read_u32(s)? as usize;
    if len > 4096 {
        return Err(Error::Protocol("failure reason too long".into()));
    }
    let mut b = vec![0u8; len];
    s.read_exact(&mut b)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A duplex stub: reads come from `input`, writes are collected in `out`.
    struct Duplex {
        input: Cursor<Vec<u8>>,
        out: Vec<u8>,
    }

    impl Duplex {
        fn new(input: Vec<u8>) -> Self {
            Duplex { input: Cursor::new(input), out: Vec::new() }
        }
    }

    impl Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.out.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn server_init(w: u16, h: u16, name: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&PixelFormat::VIEWER.encode());
        v.extend_from_slice(&(name.len() as u32).to_be_bytes());
        v.extend_from_slice(name.as_bytes());
        v
    }

    #[test]
    fn parses_and_answers_versions() {
        assert_eq!(parse_version(b"RFB 003.008\n").unwrap(), Version { major: 3, minor: 8 });
        assert!(parse_version(b"HTTP/1.1 200").is_err());
        assert_eq!(answer_version(Version { major: 3, minor: 889 }).unwrap(), Version { major: 3, minor: 8 });
        assert_eq!(answer_version(Version { major: 3, minor: 7 }).unwrap(), Version { major: 3, minor: 7 });
        assert_eq!(answer_version(Version { major: 3, minor: 5 }).unwrap(), Version { major: 3, minor: 3 });
        assert!(answer_version(Version { major: 4, minor: 0 }).is_err());
        assert_eq!(&version_bytes(Version { major: 3, minor: 8 }), b"RFB 003.008\n");
    }

    #[test]
    fn full_handshake_with_none_security_in_three_eight() {
        // version, one offered type (None), security result ok, ServerInit for a 640x480 desktop named "desk".
        let mut input = b"RFB 003.008\n".to_vec();
        input.extend_from_slice(&[1, SECURITY_NONE]);
        input.extend_from_slice(&0u32.to_be_bytes());
        input.extend_from_slice(&server_init(640, 480, "desk"));
        let mut s = Duplex::new(input);
        let init = connect(&mut s).unwrap();
        assert_eq!((init.width, init.height, init.name.as_str()), (640, 480, "desk"));
        assert_eq!(init.format, PixelFormat::VIEWER);
        // The viewer wrote its version, its security choice, and ClientInit (shared).
        assert_eq!(&s.out[..12], b"RFB 003.008\n");
        assert_eq!(s.out[12], SECURITY_NONE);
        assert_eq!(*s.out.last().unwrap(), 1);
    }

    #[test]
    fn three_three_server_chooses_security_and_sends_no_result() {
        let mut input = b"RFB 003.003\n".to_vec();
        input.extend_from_slice(&1u32.to_be_bytes());
        input.extend_from_slice(&server_init(100, 50, ""));
        let mut s = Duplex::new(input);
        let init = connect(&mut s).unwrap();
        assert_eq!((init.width, init.height), (100, 50));
        assert_eq!(&s.out[..12], b"RFB 003.003\n");
        assert_eq!(s.out.len(), 12 + 1, "version then ClientInit only");
    }

    #[test]
    fn password_only_server_is_refused() {
        let mut input = b"RFB 003.008\n".to_vec();
        input.extend_from_slice(&[1, SECURITY_VNC]);
        let mut s = Duplex::new(input);
        assert!(matches!(connect(&mut s), Err(Error::Unsupported(_))));
    }

    #[test]
    fn server_failure_reason_is_reported() {
        let mut input = b"RFB 003.008\n".to_vec();
        input.extend_from_slice(&[0, 0, 0, 0, 3]);
        input.extend_from_slice(b"no!");
        let mut s = Duplex::new(input);
        assert_eq!(connect(&mut s), Err(Error::Protocol("no!".into())));
    }

    #[test]
    fn rejects_a_failed_security_result() {
        let mut input = b"RFB 003.008\n".to_vec();
        input.extend_from_slice(&[1, SECURITY_NONE]);
        input.extend_from_slice(&1u32.to_be_bytes());
        input.extend_from_slice(&2u32.to_be_bytes());
        input.extend_from_slice(b"ab");
        let mut s = Duplex::new(input);
        assert_eq!(connect(&mut s), Err(Error::Protocol("ab".into())));
    }
}
