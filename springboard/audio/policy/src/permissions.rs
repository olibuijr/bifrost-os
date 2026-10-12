//! Permissions (microphone, capture/tap, hog) stored per app, and the input indicator. An unknown app is prompted
//! once through the shell; the answer is stored. The indicator is on while any input stream or tap runs.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Microphone,
    Capture,
    Exclusive,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Microphone => "microphone",
            Kind::Capture => "audio-capture",
            Kind::Exclusive => "audio-exclusive",
        }
    }
    fn parse(s: &str) -> Option<Kind> {
        [Kind::Microphone, Kind::Capture, Kind::Exclusive].into_iter().find(|k| k.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    /// Ask the user through the shell; the answer comes back with `answer`.
    Prompt,
}

#[derive(Clone, Debug, Default)]
pub struct Permissions {
    stored: BTreeMap<(String, &'static str), bool>,
    asked: Vec<(String, &'static str)>,
    inputs: BTreeMap<u32, String>,
}

impl Permissions {
    pub fn request(&mut self, app: &str, kind: Kind) -> Decision {
        match self.stored.get(&(app.to_string(), kind.name())) {
            Some(true) => Decision::Allow,
            Some(false) => Decision::Deny,
            None => {
                let key = (app.to_string(), kind.name());
                if self.asked.contains(&key) {
                    Decision::Deny // a prompt is already showing; this request waits as denied
                } else {
                    self.asked.push(key);
                    Decision::Prompt
                }
            }
        }
    }

    pub fn answer(&mut self, app: &str, kind: Kind, allow: bool) {
        self.asked.retain(|k| !(k.0 == app && k.1 == kind.name()));
        self.stored.insert((app.to_string(), kind.name()), allow);
    }

    /// An input stream or tap of `app` started (`stream` is the server's id) or stopped.
    pub fn input_running(&mut self, stream: u32, app: &str, on: bool) {
        if on {
            self.inputs.insert(stream, app.to_string());
        } else {
            self.inputs.remove(&stream);
        }
    }

    /// The menu-bar input indicator.
    pub fn indicator(&self) -> bool {
        !self.inputs.is_empty()
    }

    /// The apps behind the indicator, sorted and deduplicated (for its menu).
    pub fn indicator_apps(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.inputs.values().map(String::as_str).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn save(&self) -> String {
        // An app id with a tab or newline would break the line format; such ids are not stored.
        self.stored
            .iter()
            .filter(|((a, _), _)| !a.contains(|c| matches!(c, '\t' | '\n' | '\r')))
            .map(|((a, k), v)| format!("{a}\t{k}\t{}\n", u8::from(*v)))
            .collect()
    }

    pub fn load(&mut self, text: &str) {
        for l in text.lines() {
            let p: Vec<&str> = l.split('\t').collect();
            if let [app, k, v] = p[..] {
                if let Some(k) = Kind::parse(k) {
                    self.stored.insert((app.to_string(), k.name()), v == "1");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_once_then_remembers() {
        let mut p = Permissions::default();
        assert_eq!(p.request("memos", Kind::Microphone), Decision::Prompt);
        assert_eq!(p.request("memos", Kind::Microphone), Decision::Deny);
        p.answer("memos", Kind::Microphone, true);
        assert_eq!(p.request("memos", Kind::Microphone), Decision::Allow);
        assert_eq!(p.request("memos", Kind::Capture), Decision::Prompt);
        let mut q = Permissions::default();
        q.load(&p.save());
        assert_eq!(q.request("memos", Kind::Microphone), Decision::Allow);
        assert_eq!(p.save(), "memos\tmicrophone\t1\n");
    }

    #[test]
    fn indicator_follows_input_streams() {
        let mut p = Permissions::default();
        assert!(!p.indicator());
        p.input_running(7, "memos", true);
        p.input_running(8, "memos", true);
        assert!(p.indicator());
        assert_eq!(p.indicator_apps(), vec!["memos"]);
        p.input_running(7, "memos", false);
        assert!(p.indicator());
        p.input_running(8, "memos", false);
        assert!(!p.indicator());
    }

    #[test]
    fn a_denial_is_stored_and_survives_a_reload() {
        let mut p = Permissions::default();
        assert_eq!(p.request("rec", Kind::Exclusive), Decision::Prompt);
        p.answer("rec", Kind::Exclusive, false);
        assert_eq!(p.request("rec", Kind::Exclusive), Decision::Deny);
        assert_eq!(p.save(), "rec\taudio-exclusive\t0\n");
        let mut q = Permissions::default();
        q.load(&p.save());
        assert_eq!(q.request("rec", Kind::Exclusive), Decision::Deny);
    }

    #[test]
    fn prompts_are_per_app_and_kind() {
        let mut p = Permissions::default();
        assert_eq!(p.request("a", Kind::Microphone), Decision::Prompt);
        assert_eq!(p.request("b", Kind::Microphone), Decision::Prompt);
        assert_eq!(p.request("a", Kind::Capture), Decision::Prompt);
        p.answer("a", Kind::Microphone, true);
        assert_eq!(p.request("b", Kind::Microphone), Decision::Deny, "b's prompt is still showing");
        assert_eq!(p.request("a", Kind::Capture), Decision::Deny);
    }

    #[test]
    fn load_ignores_garbage_and_unknown_kinds() {
        let mut p = Permissions::default();
        p.load("junk\nx\tcamera\t1\ny\tmicrophone\nz\tmicrophone\t1\textra\nok\taudio-capture\t1\n");
        assert_eq!(p.save(), "ok\taudio-capture\t1\n");
    }

    #[test]
    fn ids_that_would_break_the_file_are_not_saved() {
        let mut p = Permissions::default();
        p.answer("bad\tid", Kind::Microphone, true);
        p.answer("bad\nid", Kind::Microphone, true);
        p.answer("good", Kind::Microphone, true);
        assert_eq!(p.save(), "good\tmicrophone\t1\n");
        assert_eq!(p.request("bad\tid", Kind::Microphone), Decision::Allow, "kept for this session");
    }

    #[test]
    fn indicator_lists_each_app_once_in_order() {
        let mut p = Permissions::default();
        p.input_running(1, "zed", true);
        p.input_running(2, "alpha", true);
        p.input_running(3, "zed", true);
        assert_eq!(p.indicator_apps(), vec!["alpha", "zed"]);
        p.input_running(9, "never", false);
        assert_eq!(p.indicator_apps().len(), 2);
    }
}
