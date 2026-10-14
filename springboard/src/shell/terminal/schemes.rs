//! Links that name a command rather than a web page: ssh://, telnet:// and x-man-page://. A link becomes the argv
//! of a new tab. Every word is checked first, so a link cannot pass an option to the command.
use super::selection::man_argv;

/// The command a scheme link runs: `ssh://[user@]host[:port]`, `telnet://host[:port]` or `x-man-page://[section/]name`.
/// None for other links and for any part that is not a plain host, user, port or page name.
pub fn link_argv(url: &str) -> Option<Vec<String>> {
    if let Some(rest) = url.strip_prefix("ssh://") {
        let (user, host, port) = authority(rest)?;
        let mut argv = vec!["ssh".to_string()];
        if let Some(port) = port {
            argv.extend(["-p".to_string(), port.to_string()]);
        }
        argv.push(match user {
            Some(user) => format!("{user}@{host}"),
            None => host.to_string(),
        });
        return Some(argv);
    }
    if let Some(rest) = url.strip_prefix("telnet://") {
        let (user, host, port) = authority(rest)?;
        if user.is_some() {
            return None;
        }
        let mut argv = vec!["telnet".to_string(), host.to_string()];
        if let Some(port) = port {
            argv.push(port.to_string());
        }
        return Some(argv);
    }
    let page = url.strip_prefix("x-man-page://")?.trim_end_matches('/');
    man_argv(&page.replace('/', " "))
}

/// The first ssh, telnet or x-man-page link in `text` that `link_argv` accepts, in reading order.
/// Closing punctuation after a link is not part of it.
pub fn first_scheme_link(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|word| word.trim_end_matches(|c: char| ".,;:!?)]}'\"".contains(c)))
        .find(|word| link_argv(word).is_some())
        .map(str::to_string)
}

/// `[user@]host[:port]` with an optional trailing slash, split into its parts; None when a part is missing,
/// starts with a dash, uses a character outside its alphabet or the port is not 1 to 65535.
fn authority(rest: &str) -> Option<(Option<&str>, &str, Option<u16>)> {
    let rest = rest.trim_end_matches('/');
    let (user, hostport) = match rest.rsplit_once('@') {
        Some((user, hostport)) => (Some(user), hostport),
        None => (None, rest),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().ok().filter(|&n| n > 0)?)),
        None => (hostport, None),
    };
    let word_ok = |w: &str| {
        !w.is_empty() && !w.starts_with('-') && w.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    if !word_ok(host) || user.is_some_and(|u| !word_ok(u)) {
        return None;
    }
    Some((user, host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn ssh_links_name_the_user_host_and_port() {
        assert_eq!(link_argv("ssh://example.org"), argv(&["ssh", "example.org"]));
        assert_eq!(link_argv("ssh://olibuijr@titan.lan/"), argv(&["ssh", "olibuijr@titan.lan"]));
        assert_eq!(link_argv("ssh://ops@10.0.0.2:2222"), argv(&["ssh", "-p", "2222", "ops@10.0.0.2"]));
    }

    #[test]
    fn telnet_links_take_a_host_and_optional_port_but_no_user() {
        assert_eq!(link_argv("telnet://switch.lan"), argv(&["telnet", "switch.lan"]));
        assert_eq!(link_argv("telnet://switch.lan:2323"), argv(&["telnet", "switch.lan", "2323"]));
        assert_eq!(link_argv("telnet://admin@switch.lan"), None);
    }

    #[test]
    fn man_page_links_read_an_optional_section_and_a_name() {
        assert_eq!(link_argv("x-man-page://printf"), argv(&["man", "printf"]));
        assert_eq!(link_argv("x-man-page://3/printf"), argv(&["man", "3", "printf"]));
        assert_eq!(link_argv("x-man-page://3/printf/"), argv(&["man", "3", "printf"]));
        assert_eq!(link_argv("x-man-page://3/a/b"), None);
        assert_eq!(link_argv("x-man-page://-k"), None);
    }

    #[test]
    fn hostile_or_malformed_links_run_nothing() {
        assert_eq!(link_argv("ssh://-oProxyCommand=x"), None);
        assert_eq!(link_argv("ssh://host;rm"), None);
        assert_eq!(link_argv("ssh://host%20x"), None);
        assert_eq!(link_argv("ssh://"), None);
        assert_eq!(link_argv("ssh://host:0"), None);
        assert_eq!(link_argv("ssh://host:99999"), None);
        assert_eq!(link_argv("telnet://-h"), None);
        assert_eq!(link_argv("https://example.org"), None);
    }

    #[test]
    fn the_first_usable_link_in_the_text_is_taken() {
        assert_eq!(first_scheme_link("see ssh://a.lan. then telnet://b.lan").as_deref(), Some("ssh://a.lan"));
        assert_eq!(first_scheme_link("ssh://-x telnet://b.lan:23,").as_deref(), Some("telnet://b.lan:23"));
        assert_eq!(first_scheme_link("https://example.org only"), None);
        assert_eq!(first_scheme_link(""), None);
    }
}
