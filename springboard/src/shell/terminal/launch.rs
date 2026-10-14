//! Links the Terminal opens: ssh and telnet connections and manual pages (the x-man-page scheme), and the manual page
//! of a selected word. Each becomes a command line typed into a new tab, so the tab stays when the command ends.
use super::*;

/// The command (words, no shell) a link asks the Terminal to run. None for another scheme or a malformed link.
pub fn command_for(url: &str) -> Option<Vec<String>> {
    if url.chars().any(char::is_control) {
        return None;
    }
    let (scheme, rest) = url.split_once(':')?;
    match scheme.to_ascii_lowercase().as_str() {
        "ssh" => {
            let (user, host, port) = split_authority(rest.strip_prefix("//")?)?;
            let mut argv = vec!["ssh".to_string()];
            if let Some(port) = port {
                argv.extend(["-p".to_string(), port.to_string()]);
            }
            argv.push(match user {
                Some(user) => format!("{user}@{host}"),
                None => host,
            });
            Some(argv)
        }
        "telnet" => {
            let (_, host, port) = split_authority(rest.strip_prefix("//")?)?;
            let mut argv = vec!["telnet".to_string(), host];
            argv.extend(port.map(|p| p.to_string()));
            Some(argv)
        }
        "x-man-page" => {
            let path = rest.strip_prefix("//").unwrap_or(rest);
            let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                [name] => man_command(None, name),
                [section, name] => man_command(Some(section), name),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `(user, host, port)` of a URL authority such as `me@host:2222` or `[::1]:22`; a password is dropped.
fn split_authority(authority: &str) -> Option<(Option<String>, String, Option<u16>)> {
    let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
    let authority = &authority[..end];
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, authority),
    };
    let user = match userinfo {
        Some(info) => {
            let name = info.split(':').next().unwrap_or("");
            if !valid_user(name) {
                return None;
            }
            Some(name.to_string())
        }
        None => None,
    };
    let (host, port_text) = match hostport.strip_prefix('[') {
        Some(v6) => {
            let (host, after) = v6.split_once(']')?;
            match after {
                "" => (host, None),
                after => (host, Some(after.strip_prefix(':')?)),
            }
        }
        None => match hostport.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (hostport, None),
        },
    };
    if !valid_host(host) {
        return None;
    }
    let port = match port_text {
        None | Some("") => None,
        Some(text) => Some(text.parse::<u16>().ok().filter(|&p| p != 0)?),
    };
    Some((user, host.to_string(), port))
}

/// A user name that cannot be taken for an option or smuggle in another word.
fn valid_user(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

/// A host name or address (IPv6 included) that cannot be taken for an option or smuggle in another word.
fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && !host.starts_with('-') && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
}

/// `man [section] name` for a page name (letters, digits and . _ + - :) and an optional section (`1`, `3p`, `n`, `l`).
fn man_command(section: Option<&str>, name: &str) -> Option<Vec<String>> {
    let name_ok = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-' | ':'));
    let section_ok = section.map_or(true, |s| {
        (1..=4).contains(&s.len()) && s.starts_with(|c: char| c.is_ascii_digit() || matches!(c, 'n' | 'l')) && s.chars().all(|c| c.is_ascii_alphanumeric())
    });
    if !name_ok || !section_ok {
        return None;
    }
    let mut argv = vec!["man".to_string()];
    argv.extend(section.map(str::to_string));
    argv.push(name.to_string());
    Some(argv)
}

/// The manual page named by the first word of `selection`: `ls`, or `ls(1)` for a page in a section.
pub fn man_for_selection(selection: &str) -> Option<Vec<String>> {
    let word = selection.split_whitespace().next()?.trim_end_matches([',', '.', ';', ':']);
    match word.strip_suffix(')').and_then(|w| w.split_once('(')) {
        Some((name, section)) => man_command(Some(section), name),
        None => man_command(None, word),
    }
}

/// One word quoted for a POSIX shell: as is when it holds only safe characters, else in single quotes.
pub fn quote_word(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '_' | '-')) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// The words as one shell command line.
pub fn quote_line(argv: &[String]) -> String {
    argv.iter().map(|w| quote_word(w)).collect::<Vec<_>>().join(" ")
}

impl Shell {
    /// Types `argv` as a command line into the active tab.
    fn terminal_type_command(&mut self, argv: &[String]) {
        let line = format!("{}\r", quote_line(argv));
        if let Some(term) = self.term.as_mut() {
            term.write(line.as_bytes());
        }
        self.terminal_ui.panel = Panel::None;
        self.dirty = true;
    }

    /// Opens a tab in the home folder and types `argv` as its first command.
    fn terminal_run_argv(&mut self, argv: &[String]) -> bool {
        if !self.new_terminal_tab(None) {
            return false;
        }
        self.terminal_type_command(argv);
        true
    }

    /// Opens a link the Terminal handles (ssh, telnet, x-man-page): brings the Terminal up and runs it in a new tab.
    /// False, and nothing opened, for a link that is not one of those or is malformed.
    pub(crate) fn terminal_open_link(&mut self, url: &str, t: f64) -> bool {
        let Some(argv) = command_for(url) else { return false };
        let had_tabs = self.term.is_some();
        let (app, front) = (super::super::AppId::Terminal, super::super::Screen::App(super::super::AppId::Terminal));
        if self.desk_active() {
            self.desk_open_or_start(app, t);
        } else if self.screen != front {
            self.open(app, t);
        }
        // opening the Terminal for the first time started a tab already; the command goes into it
        if !had_tabs && self.term.len() == 1 {
            self.terminal_type_command(&argv);
            return true;
        }
        self.terminal_run_argv(&argv)
    }

    /// Shows the manual page of the first word of the selection in a new tab.
    pub(super) fn terminal_man_page(&mut self, t: f64) {
        let selection = self.term.as_ref().and_then(|term| term.selected_text());
        let Some(argv) = selection.as_deref().and_then(man_for_selection) else {
            self.terminal_ui.message = "Select a command name first".into();
            return;
        };
        if self.terminal_run_argv(&argv) {
            self.terminal_close_panel(t);
        }
    }

    /// Runs `man -k` for the selected word in a new tab (Search Man Pages for Selection).
    pub(super) fn terminal_man_search(&mut self, t: f64) {
        let selection = self.term.as_ref().and_then(|term| term.selected_text());
        let Some(argv) = selection.as_deref().and_then(man_search_for_selection) else {
            self.terminal_ui.message = "Select a command name first".into();
            return;
        };
        if self.terminal_run_argv(&argv) {
            self.terminal_close_panel(t);
        }
    }

    /// Shows Look Up for the selected word in the popover (Look Up Selection).
    pub(super) fn terminal_look_up(&mut self, t: f64) {
        let term = self
            .term
            .as_ref()
            .and_then(|term| term.selected_text())
            .and_then(|text| bifrost_ui::look_up::term_of(&text));
        match term {
            Some(term) => {
                self.open_look_up(term);
                self.terminal_close_panel(t);
            }
            None => self.terminal_ui.message = "Select a word first".into(),
        }
    }

    /// Brings the Terminal up in a new tab in `dir` (New Terminal at Folder, from the Services menu). The first tab of a
    /// Terminal that was not running is used for the folder.
    pub(crate) fn terminal_open_in_folder(&mut self, dir: &str, t: f64) {
        let (app, front) = (super::super::AppId::Terminal, super::super::Screen::App(super::super::AppId::Terminal));
        let had_tabs = self.term.len();
        if self.desk_active() {
            self.desk_open_or_start(app, t);
        } else if self.screen != front {
            self.open(app, t);
        }
        if had_tabs == 0 && self.term.len() == 1 {
            let quoted = dir.replace('\'', "'\\''");
            if let Some(term) = self.term.as_mut() {
                term.write(format!("cd '{quoted}'\r").as_bytes());
            }
        } else {
            self.new_terminal_tab(Some(dir.to_string()));
        }
        self.terminal_ui.message.clear();
        self.dirty = true;
    }

    /// Opens a link tapped in the output: a web link in the browser, the others (ssh, telnet, manual pages) in a new tab.
    pub(crate) fn terminal_open_tapped(&mut self, url: &str, t: f64) {
        if url.starts_with("http://") || url.starts_with("https://") {
            self.terminal_ui.message = match self.desktop_open_url(url, t) {
                Ok(()) => "Opened link".into(),
                Err(e) => format!("Could not open link: {e}"),
            };
        } else if !self.terminal_open_link(url, t) {
            self.terminal_ui.message = "That link cannot be opened".into();
        }
    }
}

/// `man -k word` for the first word of `selection`: a keyword search over the page names and descriptions.
pub fn man_search_for_selection(selection: &str) -> Option<Vec<String>> {
    let word = selection.split_whitespace().next()?.trim_end_matches([',', '.', ';', ':']);
    let plain = !word.is_empty()
        && word.len() <= 64
        && word.starts_with(|c: char| c.is_ascii_alphanumeric())
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-' | ':'));
    plain.then(|| vec!["man".to_string(), "-k".to_string(), word.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(url: &str) -> Option<Vec<String>> {
        command_for(url)
    }

    fn v(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn ssh_links_name_user_host_and_port() {
        assert_eq!(words("ssh://example.org"), v(&["ssh", "example.org"]));
        assert_eq!(words("ssh://me@example.org:2222/"), v(&["ssh", "-p", "2222", "me@example.org"]));
        assert_eq!(words("SSH://me:secret@host.lan/path?x=1"), v(&["ssh", "me@host.lan"]), "the password is dropped, the path ignored");
        assert_eq!(words("ssh://[::1]:22"), v(&["ssh", "-p", "22", "::1"]));
        assert_eq!(words("ssh://host:"), v(&["ssh", "host"]), "an empty port is none");
    }

    #[test]
    fn telnet_links_name_host_and_port() {
        assert_eq!(words("telnet://bbs.example.org"), v(&["telnet", "bbs.example.org"]));
        assert_eq!(words("telnet://bbs.example.org:2323"), v(&["telnet", "bbs.example.org", "2323"]));
    }

    #[test]
    fn malformed_or_hostile_links_run_nothing() {
        for bad in [
            "ssh://-oProxyCommand=evil",
            "ssh://me@-bad",
            "ssh://-me@host",
            "ssh://host:0",
            "ssh://host:99999",
            "ssh://host:ab",
            "ssh://ho st",
            "ssh://host;rm",
            "ssh://",
            "ssh:host",
            "ssh://[::1",
            "ssh://host\n",
            "telnet://$(id)",
            "https://example.org",
            "mailto:a@b.c",
            "noscheme",
        ] {
            assert_eq!(words(bad), None, "{bad}");
        }
    }

    #[test]
    fn manual_page_links_name_the_page_and_section() {
        assert_eq!(words("x-man-page://ls"), v(&["man", "ls"]));
        assert_eq!(words("x-man-page://1/ls"), v(&["man", "1", "ls"]));
        assert_eq!(words("x-man-page://3p/printf?x#y"), v(&["man", "3p", "printf"]));
        assert_eq!(words("x-man-page:ls"), v(&["man", "ls"]));
        assert_eq!(words("x-man-page://-k/ls"), None, "a section is a digit or n or l");
        assert_eq!(words("x-man-page://1/-k"), None, "a page name does not start with a dash");
        assert_eq!(words("x-man-page://1/2/3"), None);
        assert_eq!(words("x-man-page://"), None);
        assert_eq!(words("x-man-page://12345/ls"), None, "sections are at most four characters");
    }

    #[test]
    fn the_first_word_of_a_selection_names_a_manual_page() {
        assert_eq!(man_for_selection("ls -la"), v(&["man", "ls"]));
        assert_eq!(man_for_selection("  printf(3), see also"), v(&["man", "3", "printf"]));
        assert_eq!(man_for_selection("grep."), v(&["man", "grep"]));
        assert_eq!(man_for_selection("git-commit"), v(&["man", "git-commit"]));
        assert_eq!(man_for_selection("--help"), None);
        assert_eq!(man_for_selection("$(id)"), None);
        assert_eq!(man_for_selection("   "), None);
        assert_eq!(man_for_selection(""), None);
    }

    #[test]
    fn a_keyword_search_takes_the_first_plain_word() {
        assert_eq!(man_search_for_selection("printf the"), v(&["man", "-k", "printf"]));
        assert_eq!(man_search_for_selection("tar,"), v(&["man", "-k", "tar"]));
        assert_eq!(man_search_for_selection("--help"), None);
        assert_eq!(man_search_for_selection("$(id)"), None);
        assert_eq!(man_search_for_selection("a/b"), None, "a path is not a page name");
        assert_eq!(man_search_for_selection("  "), None);
    }

    #[test]
    fn words_are_quoted_for_the_shell() {
        assert_eq!(quote_word("me@host.lan"), "me@host.lan");
        assert_eq!(quote_word("a b"), "'a b'");
        assert_eq!(quote_word("it's"), "'it'\\''s'");
        assert_eq!(quote_word(""), "''");
        let argv = command_for("ssh://me@host:2222").unwrap();
        assert_eq!(quote_line(&argv), "ssh -p 2222 me@host");
    }
}
