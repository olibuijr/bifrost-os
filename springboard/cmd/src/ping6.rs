//! `ping6`: pings an IPv6 host. The installed `ping` (iputils) does this with `-6`, so `ping6` runs it with that flag in
//! front of the arguments it was given. Only the name is new; the probe and its output are `ping`'s.

pub const USAGE: &str = "usage: ping6 [ping options] host\nRuns ping -6: pings an IPv6 address or name.";

/// The arguments for `ping`: `-6` first, then the caller's arguments unchanged. A `-4` flag from the caller is refused,
/// since the two families cannot be asked for together.
pub fn ping_args(args: &[String]) -> Result<Vec<String>, String> {
    if args.iter().any(|a| a == "-4") {
        return Err("-4 cannot be combined with ping6, which pings IPv6 only".into());
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    out.push("-6".to_string());
    out.extend(args.iter().cloned());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ping_gets_the_ipv6_flag_first() {
        assert_eq!(ping_args(&args(&["-c", "3", "::1"])), Ok(args(&["-6", "-c", "3", "::1"])));
        assert_eq!(ping_args(&args(&["example.com"])), Ok(args(&["-6", "example.com"])));
    }

    #[test]
    fn ipv4_is_refused() {
        assert!(ping_args(&args(&["-4", "10.0.0.1"])).unwrap_err().contains("-4"));
    }

    #[test]
    fn no_arguments_still_pings_the_flag_alone() {
        assert_eq!(ping_args(&[]), Ok(args(&["-6"])));
    }
}
