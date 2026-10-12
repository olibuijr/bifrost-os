//! Print dialog plug-ins and their panels: the model behind the accessory views of the print and page setup dialogs.
//!
//! A [`PlugIn`] is a named bundle of [`PrintPanel`]s. Each panel lists typed [`PanelOption`]s: a choice from a list, a
//! toggle, or a bounded number. [`PlugIns::register`] keeps panel ids and option keys unique across every plug-in and
//! refuses any key that [`PrintInfo`](crate::print::PrintInfo) already writes (paper, copies, margins, pages). The values
//! render as CUPS `-o key=value` pairs through [`PlugIns::lp_options`], which
//! [`PrintOperation::with_options`](crate::print::PrintOperation::with_options) appends to the `lp` call.
//!
//! [`Placement`] says which dialog hosts a panel: the print dialog or the page setup dialog. The shell does not draw
//! either dialog yet (see docs/bifrost-ui.md "Printing"), so an app shows its panels with its own kit controls and reads
//! and writes values by key. Nothing here touches the system: it is a pure model with host tests.

/// Keys that [`PrintInfo::lp_options`](crate::print::PrintInfo::lp_options) already writes. A plug-in cannot set them.
pub const RESERVED_KEYS: [&str; 9] =
    ["media", "landscape", "portrait", "copies", "page-top", "page-left", "page-bottom", "page-right", "page-ranges"];

/// The dialog that hosts a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The print dialog, shown before the job is submitted.
    PrintDialog,
    /// The page setup dialog, shown for paper and orientation.
    PageLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// The id or key is empty, too long, or has characters outside `a-z A-Z 0-9 - _`.
    BadKey(String),
    /// The key is one that the print info already writes.
    ReservedKey(String),
    /// Two options (in any plug-in) share the key.
    DuplicateKey(String),
    /// Two plug-ins or two panels share the id.
    DuplicateId(String),
    /// No option has this key.
    UnknownKey(String),
    /// The value is a different kind from the option (a toggle set to a number, for example).
    KindMismatch(String),
    /// A choice index, a number, or a default is outside the option's range.
    OutOfRange(String),
    /// A choice list is empty or holds an empty string, or a choice label is empty.
    Empty(String),
}

/// What an option accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// One of the listed values, selected by index.
    Choice(Vec<String>),
    /// On or off.
    Toggle,
    /// A whole number from `min` to `max`, inclusive.
    Number { min: i32, max: i32 },
}

/// The current setting of an option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Choice(usize),
    Toggle(bool),
    Number(i32),
}

/// One setting in a panel. `key` is the CUPS option name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelOption {
    pub key: String,
    pub label: String,
    pub kind: Kind,
    pub value: Value,
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl PanelOption {
    /// A choice from `choices`, starting at index `default`.
    pub fn choice(key: &str, label: &str, choices: &[&str], default: usize) -> Result<Self, PanelError> {
        if !valid_key(key) {
            return Err(PanelError::BadKey(key.into()));
        }
        if choices.is_empty() || choices.iter().any(|c| c.is_empty()) {
            return Err(PanelError::Empty(key.into()));
        }
        if default >= choices.len() {
            return Err(PanelError::OutOfRange(key.into()));
        }
        let kind = Kind::Choice(choices.iter().map(|c| c.to_string()).collect());
        Ok(PanelOption { key: key.into(), label: label.into(), kind, value: Value::Choice(default) })
    }

    /// An on or off option.
    pub fn toggle(key: &str, label: &str, default: bool) -> Result<Self, PanelError> {
        if !valid_key(key) {
            return Err(PanelError::BadKey(key.into()));
        }
        Ok(PanelOption { key: key.into(), label: label.into(), kind: Kind::Toggle, value: Value::Toggle(default) })
    }

    /// A whole number from `min` to `max`, starting at `default`.
    pub fn number(key: &str, label: &str, min: i32, max: i32, default: i32) -> Result<Self, PanelError> {
        if !valid_key(key) {
            return Err(PanelError::BadKey(key.into()));
        }
        if min > max || !(min..=max).contains(&default) {
            return Err(PanelError::OutOfRange(key.into()));
        }
        Ok(PanelOption { key: key.into(), label: label.into(), kind: Kind::Number { min, max }, value: Value::Number(default) })
    }

    /// The CUPS value of the current setting: the choice text, `true` or `false`, or the number.
    pub fn lp_value(&self) -> String {
        match (&self.kind, &self.value) {
            (Kind::Choice(c), Value::Choice(i)) => c.get(*i).cloned().unwrap_or_default(),
            (_, Value::Toggle(b)) => b.to_string(),
            (_, Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    }

    /// The text a panel shows for the current setting: the choice text, `On` or `Off`, or the number.
    pub fn display(&self) -> String {
        match &self.value {
            Value::Choice(_) => self.lp_value(),
            Value::Toggle(true) => "On".into(),
            Value::Toggle(false) => "Off".into(),
            Value::Number(n) => n.to_string(),
        }
    }

    /// Checks that `value` has this option's kind and is in range.
    fn accepts(&self, value: &Value) -> Result<(), PanelError> {
        match (&self.kind, value) {
            (Kind::Choice(c), Value::Choice(i)) if *i < c.len() => Ok(()),
            (Kind::Choice(_), Value::Choice(_)) => Err(PanelError::OutOfRange(self.key.clone())),
            (Kind::Toggle, Value::Toggle(_)) => Ok(()),
            (Kind::Number { min, max }, Value::Number(n)) if (*min..=*max).contains(n) => Ok(()),
            (Kind::Number { .. }, Value::Number(_)) => Err(PanelError::OutOfRange(self.key.clone())),
            _ => Err(PanelError::KindMismatch(self.key.clone())),
        }
    }
}

/// A panel of settings shown in one dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintPanel {
    pub id: String,
    pub title: String,
    pub placement: Placement,
    /// The options in display order. Their order is the order of [`PlugIns::lp_options`].
    pub options: Vec<PanelOption>,
}

impl PrintPanel {
    pub fn new(id: &str, title: &str, placement: Placement) -> Self {
        PrintPanel { id: id.into(), title: title.into(), placement, options: Vec::new() }
    }

    /// Appends an option.
    pub fn option(mut self, option: PanelOption) -> Self {
        self.options.push(option);
        self
    }

    /// The option with this key, if the panel has it.
    pub fn get(&self, key: &str) -> Option<&PanelOption> {
        self.options.iter().find(|o| o.key == key)
    }
}

/// A named bundle of panels, the unit an app registers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlugIn {
    pub id: String,
    pub name: String,
    pub panels: Vec<PrintPanel>,
}

impl PlugIn {
    pub fn new(id: &str, name: &str) -> Self {
        PlugIn { id: id.into(), name: name.into(), panels: Vec::new() }
    }

    /// Appends a panel.
    pub fn panel(mut self, panel: PrintPanel) -> Self {
        self.panels.push(panel);
        self
    }
}

/// The registered plug-ins, in registration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlugIns {
    plugins: Vec<PlugIn>,
}

impl PlugIns {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a plug-in. Nothing changes when it is refused.
    pub fn register(&mut self, plugin: PlugIn) -> Result<(), PanelError> {
        self.check(&plugin)?;
        self.plugins.push(plugin);
        Ok(())
    }

    fn check(&self, plugin: &PlugIn) -> Result<(), PanelError> {
        if !valid_key(&plugin.id) {
            return Err(PanelError::BadKey(plugin.id.clone()));
        }
        if self.plugins.iter().any(|p| p.id == plugin.id) {
            return Err(PanelError::DuplicateId(plugin.id.clone()));
        }
        let all: Vec<&PrintPanel> = self.plugins.iter().flat_map(|p| p.panels.iter()).chain(plugin.panels.iter()).collect();
        let mut ids: Vec<&str> = Vec::new();
        let mut keys: Vec<&str> = Vec::new();
        for panel in all {
            if ids.contains(&panel.id.as_str()) {
                return Err(PanelError::DuplicateId(panel.id.clone()));
            }
            ids.push(panel.id.as_str());
            for o in &panel.options {
                if !valid_key(&o.key) {
                    return Err(PanelError::BadKey(o.key.clone()));
                }
                if RESERVED_KEYS.contains(&o.key.as_str()) {
                    return Err(PanelError::ReservedKey(o.key.clone()));
                }
                if keys.contains(&o.key.as_str()) {
                    return Err(PanelError::DuplicateKey(o.key.clone()));
                }
                keys.push(o.key.as_str());
            }
        }
        Ok(())
    }

    pub fn plugins(&self) -> &[PlugIn] {
        &self.plugins
    }

    fn all_panels(&self) -> impl Iterator<Item = &PrintPanel> {
        self.plugins.iter().flat_map(|p| p.panels.iter())
    }

    /// The panels a dialog hosts, in registration order.
    pub fn panels(&self, placement: Placement) -> Vec<&PrintPanel> {
        self.all_panels().filter(|p| p.placement == placement).collect()
    }

    /// The option with this key, from any panel.
    pub fn get(&self, key: &str) -> Option<&PanelOption> {
        self.all_panels().find_map(|p| p.get(key))
    }

    /// Sets the value of the option with this key. Nothing changes when the key is unknown or the value is refused.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), PanelError> {
        for plugin in &mut self.plugins {
            for panel in &mut plugin.panels {
                if let Some(option) = panel.options.iter_mut().find(|o| o.key == key) {
                    option.accepts(&value)?;
                    option.value = value;
                    return Ok(());
                }
            }
        }
        Err(PanelError::UnknownKey(key.into()))
    }

    /// Every option as `-o key=value` pairs, in registration order, ready for [`PrintOperation::with_options`](crate::print::PrintOperation::with_options).
    pub fn lp_options(&self) -> Vec<String> {
        let mut out = Vec::new();
        for panel in self.all_panels() {
            for o in &panel.options {
                out.push("-o".into());
                out.push(format!("{}={}", o.key, o.lp_value()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::PrintOperation;

    fn layout_plugin() -> PlugIn {
        let panel = PrintPanel::new("layout", "Layout", Placement::PageLayout)
            .option(PanelOption::choice("sides", "Two-Sided", &["one-sided", "two-sided-long-edge", "two-sided-short-edge"], 0).unwrap())
            .option(PanelOption::number("quality", "Quality", 1, 5, 3).unwrap())
            .option(PanelOption::toggle("fit", "Fit to page", false).unwrap());
        PlugIn::new("layout-tools", "Layout tools").panel(panel)
    }

    #[test]
    fn register_refuses_duplicate_ids_duplicate_keys_and_reserved_keys() {
        let mut p = PlugIns::new();
        p.register(layout_plugin()).unwrap();
        let dup_panel = PlugIn::new("other", "Other").panel(PrintPanel::new("layout", "Again", Placement::PrintDialog));
        assert_eq!(p.register(dup_panel), Err(PanelError::DuplicateId("layout".into())));
        let dup_key = PlugIn::new("other", "Other")
            .panel(PrintPanel::new("extra", "Extra", Placement::PrintDialog).option(PanelOption::toggle("fit", "Fit", true).unwrap()));
        assert_eq!(p.register(dup_key), Err(PanelError::DuplicateKey("fit".into())));
        let reserved = PlugIn::new("copies-plugin", "Copies")
            .panel(PrintPanel::new("c", "C", Placement::PrintDialog).option(PanelOption::toggle("copies", "Copies", true).unwrap()));
        assert_eq!(p.register(reserved), Err(PanelError::ReservedKey("copies".into())));
        assert_eq!(p.register(PlugIn::new("", "Empty id")), Err(PanelError::BadKey("".into())));
        assert_eq!(p.plugins().len(), 1, "a refused plug-in is not kept");
    }

    #[test]
    fn option_constructors_reject_bad_keys_and_ranges() {
        assert_eq!(PanelOption::toggle("bad key", "x", true), Err(PanelError::BadKey("bad key".into())));
        assert_eq!(PanelOption::choice("c", "x", &[], 0), Err(PanelError::Empty("c".into())));
        assert_eq!(PanelOption::choice("c", "x", &["a", ""], 0), Err(PanelError::Empty("c".into())));
        assert_eq!(PanelOption::choice("c", "x", &["a", "b"], 2), Err(PanelError::OutOfRange("c".into())));
        assert_eq!(PanelOption::number("n", "x", 5, 1, 3), Err(PanelError::OutOfRange("n".into())));
        assert_eq!(PanelOption::number("n", "x", 1, 5, 0), Err(PanelError::OutOfRange("n".into())));
    }

    #[test]
    fn set_checks_the_key_the_kind_and_the_range() {
        let mut p = PlugIns::new();
        p.register(layout_plugin()).unwrap();
        assert_eq!(p.set("quality", Value::Number(9)), Err(PanelError::OutOfRange("quality".into())));
        assert_eq!(p.set("quality", Value::Toggle(true)), Err(PanelError::KindMismatch("quality".into())));
        assert_eq!(p.set("sides", Value::Choice(3)), Err(PanelError::OutOfRange("sides".into())));
        assert_eq!(p.set("missing", Value::Toggle(true)), Err(PanelError::UnknownKey("missing".into())));
        assert_eq!(p.get("quality").unwrap().lp_value(), "3", "refused values change nothing");
        p.set("sides", Value::Choice(2)).unwrap();
        assert_eq!(p.get("sides").unwrap().lp_value(), "two-sided-short-edge");
    }

    #[test]
    fn lp_options_follow_registration_and_panel_order() {
        let mut p = PlugIns::new();
        p.register(layout_plugin()).unwrap();
        assert_eq!(p.lp_options().join(" "), "-o sides=one-sided -o quality=3 -o fit=false");
        p.set("sides", Value::Choice(1)).unwrap();
        p.set("quality", Value::Number(4)).unwrap();
        p.set("fit", Value::Toggle(true)).unwrap();
        assert_eq!(p.lp_options().join(" "), "-o sides=two-sided-long-edge -o quality=4 -o fit=true");
    }

    #[test]
    fn placement_selects_the_dialog_that_hosts_a_panel() {
        let mut p = PlugIns::new();
        p.register(layout_plugin()).unwrap();
        let output = PlugIn::new("output", "Output")
            .panel(PrintPanel::new("output", "Output", Placement::PrintDialog).option(PanelOption::toggle("collate", "Collate", true).unwrap()));
        p.register(output).unwrap();
        let page: Vec<&str> = p.panels(Placement::PageLayout).iter().map(|x| x.id.as_str()).collect();
        let dialog: Vec<&str> = p.panels(Placement::PrintDialog).iter().map(|x| x.id.as_str()).collect();
        assert_eq!(page, ["layout"]);
        assert_eq!(dialog, ["output"]);
        assert_eq!(p.get("collate").unwrap().display(), "On");
    }

    #[test]
    fn display_shows_on_off_choice_text_and_numbers() {
        let t = PanelOption::toggle("t", "T", false).unwrap();
        assert_eq!(t.display(), "Off");
        let c = PanelOption::choice("c", "C", &["x", "y"], 1).unwrap();
        assert_eq!(c.display(), "y");
        let n = PanelOption::number("n", "N", 0, 10, 7).unwrap();
        assert_eq!(n.display(), "7");
    }

    #[test]
    fn plug_in_options_reach_the_lp_call_before_the_document() {
        let mut p = PlugIns::new();
        p.register(layout_plugin()).unwrap();
        p.set("fit", Value::Toggle(true)).unwrap();
        let args = PrintOperation::new("/tmp/report.pdf").with_options(p.lp_options()).lp_args();
        let tail: Vec<&str> = args[args.len() - 8..].iter().map(|s| s.as_str()).collect();
        assert_eq!(tail, ["-o", "sides=one-sided", "-o", "quality=3", "-o", "fit=true", "--", "/tmp/report.pdf"]);
    }
}
