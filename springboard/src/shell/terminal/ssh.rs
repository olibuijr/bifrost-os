//! SSH connections: config file, connect, the add/edit form and ssh import.
use super::*;

impl Shell {
    pub(super) fn terminal_config_path(&self) -> PathBuf {
        self.data_dir.join("Terminal/ssh/config")
    }
    pub(super) fn save_terminal_config(&self, config: &Config) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let path = self.terminal_config_path();
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let temp = path.with_extension("tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(config.to_text().as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temp, &path)?;
        // the ssh child runs as the desktop user (term::identity), so the config must be theirs to read
        if let Ok(Some(l)) = crate::term::identity::current() {
            std::os::unix::fs::chown(dir, Some(l.uid), Some(l.gid))?;
            std::os::unix::fs::chown(&path, Some(l.uid), Some(l.gid))?;
        }
        Ok(())
    }
    pub(super) fn terminal_connect(&mut self, alias: &str, t: f64) {
        if !self.terminal_ui.config_loaded {
            return;
        }
        if !termcore::sshconfig::safe_alias(alias) {
            return;
        }
        let quote =
            |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        let config = self.terminal_config_path();
        if let Err(e) = self.save_terminal_config(&self.terminal_ui.config) {
            self.terminal_ui.message = e.to_string();
            return;
        }
        if !self.new_terminal_tab(None) {
            return;
        }
        let line = format!(
            "ssh -F {} -o UserKnownHostsFile={} {}\r",
            quote(&config),
            quote(&config.with_file_name("known_hosts")),
            alias
        );
        if let Some(term) = self.term.as_mut() {
            term.write(line.as_bytes());
        }
        self.terminal_ui.panel = Panel::None;
        self.set_keyboard(true, t);
    }
    pub(super) fn terminal_save_form(&mut self, t: f64) {
        if !self.terminal_ui.config_loaded {
            return;
        }
        let entry = self.terminal_ui.entry();
        let mut config = self.terminal_ui.config.clone();
        let result = match self.terminal_ui.editing.as_deref() {
            Some(alias) => config.update(alias, &entry),
            None => config.add(&entry),
        };
        match result.map_err(|e| e.to_string()).and_then(|_| {
            self.save_terminal_config(&config)
                .map_err(|e| e.to_string())
        }) {
            Ok(()) => {
                self.terminal_ui.config = config;
                self.terminal_ui.panel = Panel::Connections;
                self.terminal_ui.message = "Connection saved".into();
                self.set_keyboard(false, t);
            }
            Err(e) => self.terminal_ui.message = e,
        }
    }
    pub(super) fn terminal_import(&mut self) {
        if !self.terminal_ui.config_loaded {
            return;
        }
        let Some(source) = clipboard::text() else {
            self.terminal_ui.message =
                "Copy an SSH config or its file path, then tap Import".into();
            return;
        };
        let imported = if source.trim().starts_with('/') && !source.contains('\n') {
            match termcore::sshconfig::read_file(std::path::Path::new(source.trim())) {
                Ok(s) => termcore::sshconfig::rebase_identity_files(
                    &s,
                    std::path::Path::new(source.trim())
                        .parent()
                        .unwrap()
                        .to_str()
                        .unwrap_or(""),
                ),
                Err(e) => {
                    self.terminal_ui.message = e.to_string();
                    return;
                }
            }
        } else {
            source
        };
        let entries = Config::parse(&imported).entries();
        if self.terminal_ui.config.to_text().trim().is_empty() && !entries.is_empty() {
            let config = Config::parse(&imported);
            match self.save_terminal_config(&config) {
                Ok(()) => {
                    self.terminal_ui.config = config;
                    self.terminal_ui.message = format!("Imported {} connections", entries.len());
                }
                Err(e) => self.terminal_ui.message = e.to_string(),
            }
            return;
        }
        let mut config = self.terminal_ui.config.clone();
        let mut count = 0;
        for entry in entries {
            if config.get(&entry.alias).is_none() {
                match config.add(&entry) {
                    Ok(()) => count += 1,
                    Err(e) => {
                        self.terminal_ui.message = e.to_string();
                        return;
                    }
                }
            }
        }
        if count == 0 {
            self.terminal_ui.message = "No new named connections in the clipboard".into();
            return;
        }
        match self.save_terminal_config(&config) {
            Ok(()) => {
                self.terminal_ui.config = config;
                self.terminal_ui.message = format!("Imported {count} connections");
            }
            Err(e) => self.terminal_ui.message = e.to_string(),
        }
    }
}
