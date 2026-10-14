//! Agent control of Term: the `term <verb> key=value ...` line on the shell control socket
//! (`bare/control.rs`, `Request::Term`), driven by the MCP `term` tool (docs/bifrost-mcp.md).
//! Replies are plain text; values that may hold spaces are percent-encoded. `wait_for` is polled by the MCP
//! side with `read_screen`, because the shell loop answers one request at a time and must not block.
use super::*;
use crate::bare::control::pct_encode;
use crate::term::control::keys_bytes;
use crate::shell::apps::AppId;

/// The optional `tab` index argument.
fn tab_arg(args: &[(String, String)]) -> Result<Option<usize>, String> {
    match arg(args, "tab") {
        None | Some("") => Ok(None),
        Some(v) => v.parse().map(Some).map_err(|_| format!("tab must be a tab index, got {v:?}")),
    }
}

fn arg<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn num(args: &[(String, String)], key: &str) -> Result<Option<usize>, String> {
    arg(args, key).map(|v| v.parse().map_err(|_| format!("{key} must be a number, got {v:?}"))).transpose()
}

/// Theme index for a profile name (case-insensitive) or index.
pub(super) fn profile_index(name: &str) -> Option<usize> {
    let themes = &termcore::palette::THEMES;
    name.trim().parse::<usize>().ok().filter(|i| *i < themes.len()).or_else(|| themes.iter().position(|t| t.name.eq_ignore_ascii_case(name.trim())))
}

impl Shell {
    fn term_index(&self, args: &[(String, String)]) -> Result<usize, String> {
        if self.term.is_empty() {
            return Err("Term has no tabs; use new_tab".into());
        }
        let i = tab_arg(args)?.unwrap_or_else(|| self.term.active_index());
        if i < self.term.len() { Ok(i) } else { Err(format!("no such tab {i} (0..{})", self.term.len() - 1)) }
    }

    /// One Term control request. Returns the reply body.
    pub fn term_control(&mut self, verb: &str, args: &[(String, String)], t: f64) -> Result<String, String> {
        match verb {
            "state" => {
                let mut out = format!("tabs={}\ncurrent={}\n", self.term.len(), self.term.active_index());
                let themes = &termcore::palette::THEMES;
                let theme = self.terminal_ui.settings.theme.min(themes.len() - 1);
                out += &format!("profile={}\ntheme={theme}\nzoom={}\n", pct_encode(themes[theme].name), self.terminal_ui.settings.zoom);
                out += &format!("profiles={}\n", themes.iter().map(|t| pct_encode(t.name)).collect::<Vec<_>>().join(","));
                for (i, term) in self.term.iter().enumerate() {
                    let (cols, rows) = term.size();
                    out += &format!(
                        "tab={i} current={} title={} cwd={} job={} cols={cols} rows={rows} alive={} unread={}\n",
                        u8::from(i == self.term.active_index()),
                        pct_encode(&term.title(i)),
                        pct_encode(&term.cwd().unwrap_or_default()),
                        pct_encode(&term.job().unwrap_or_default()),
                        u8::from(term.alive()),
                        u8::from(term.unread),
                    );
                }
                Ok(out)
            }
            "new_tab" => {
                let before = self.term.len();
                let cwd = arg(args, "cwd").filter(|d| !d.is_empty());
                if let Some(d) = cwd {
                    if !std::path::Path::new(d).is_dir() {
                        return Err(format!("cwd {d:?} is not a folder"));
                    }
                }
                if before == 0 && self.desk_active() {
                    self.desk_open(AppId::Terminal, t);
                }
                if before == 0 && self.term.len() == 1 {
                    // opening the window started its first shell: use it
                    if let (Some(d), Some(term)) = (cwd, self.term.as_mut()) {
                        term.write(format!("cd '{}'\r", d.replace('\'', "'\\''")).as_bytes());
                    }
                } else if !self.new_terminal_tab(cwd.map(str::to_string)) {
                    return Err(self.terminal_ui.message.clone());
                }
                if let (Some(c), Some(term)) = (arg(args, "command").filter(|c| !c.is_empty()), self.term.as_mut()) {
                    term.write(format!("{c}\r").as_bytes());
                }
                self.dirty = true;
                Ok(format!("ok tab={}\n", self.term.active_index()))
            }
            "select_tab" => {
                let i = num(args, "tab")?.ok_or("tab is required")?;
                if i >= self.term.len() {
                    return Err(format!("no such tab {i}"));
                }
                self.term.select(i);
                self.dirty = true;
                Ok(format!("ok tab={i}\n"))
            }
            "close_tab" => {
                let i = self.term_index(args)?;
                self.close_terminal_tab(i);
                self.dirty = true;
                Ok(format!("ok tabs={}\n", self.term.len()))
            }
            "send_text" => {
                let i = self.term_index(args)?;
                let text = arg(args, "text").ok_or("text is required")?;
                // a newline is Enter, as typing it would be
                let bytes = text.replace("\r\n", "\r").replace('\n', "\r");
                self.term.get_mut(i).ok_or("tab is gone")?.write(bytes.as_bytes());
                Ok(format!("ok bytes={}\n", bytes.len()))
            }
            "send_keys" => {
                let i = self.term_index(args)?;
                let term = self.term.get_mut(i).ok_or("tab is gone")?;
                let bytes = keys_bytes(arg(args, "keys").ok_or("keys is required")?, term.app_cursor_mode())?;
                term.write(&bytes);
                Ok(format!("ok bytes={}\n", bytes.len()))
            }
            "read" => {
                let i = self.term_index(args)?;
                let term = self.term.get(i).ok_or("tab is gone")?;
                let (x, y) = term.cursor_pos();
                let (cols, rows) = term.size();
                let text = term.screen_text(num(args, "scrollback_lines")?.unwrap_or(0).min(100_000));
                Ok(format!("tab={i}\ncursor={x},{y}\nsize={cols}x{rows}\nalive={}\n---\n{text}", u8::from(term.alive())))
            }
            "resize" => {
                let i = self.term_index(args)?;
                let (cols, rows) = (num(args, "cols")?, num(args, "rows")?);
                let term = self.term.get_mut(i).ok_or("tab is gone")?;
                let (c, r) = term.size();
                term.resize_cells(cols.unwrap_or(c).clamp(10, 500), rows.unwrap_or(r).clamp(2, 200));
                let (c, r) = term.size();
                self.dirty = true;
                Ok(format!("ok size={c}x{r}\n"))
            }
            "set_profile" => {
                let name = arg(args, "name").ok_or("name is required")?;
                let theme = profile_index(name).ok_or_else(|| format!("unknown profile {name:?}; see state"))?;
                self.terminal_ui.settings.theme = theme;
                self.terminal_ui.profile = None;
                self.terminal_appearance();
                self.dirty = true;
                Ok(format!("ok profile={}\n", pct_encode(termcore::palette::THEMES[theme].name)))
            }
            "clear" => {
                let i = self.term_index(args)?;
                let term = self.term.get_mut(i).ok_or("tab is gone")?;
                term.clear_history();
                if matches!(arg(args, "screen"), Some("1" | "true")) {
                    term.write(b"\x0c");
                }
                self.dirty = true;
                Ok("ok\n".into())
            }
            v => Err(format!("unknown term verb {v:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_resolve_by_name_or_index() {
        assert_eq!(profile_index("0"), Some(0));
        assert_eq!(profile_index(termcore::palette::THEMES[1].name), Some(1));
        assert_eq!(profile_index(&termcore::palette::THEMES[1].name.to_uppercase()), Some(1));
        assert_eq!(profile_index("99"), None);
        assert_eq!(profile_index("nope"), None);
    }

    #[test]
    fn tab_argument_parses() {
        let a = |v: &str| vec![("tab".to_string(), v.to_string())];
        assert_eq!(tab_arg(&a("3")), Ok(Some(3)));
        assert_eq!(tab_arg(&[]), Ok(None));
        assert!(tab_arg(&a("x")).is_err());
    }
}
