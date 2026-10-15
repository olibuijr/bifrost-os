//! Control widgets: the controls an app adds to the system control panel. A control has an identifier that is unique
//! in the registry, a display name and a kind. A button runs the app's action when the user presses it. A toggle holds
//! an on or off value. The registry is the value source the panel reads, and the panel's presses reach it through
//! `press`, `toggle` and `set_value`. Pure: nothing is drawn or stored outside the registry.

/// The longest identifier a control may have, in bytes.
pub const MAX_IDENTIFIER_LEN: usize = 128;

/// What a control does when the user uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// Runs the app's action each time it is pressed.
    Button,
    /// Holds an on or off value.
    Toggle,
}

/// The configuration an app declares for one control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticControlConfiguration {
    /// The name the registry and the app refer to the control by. ASCII letters and digits, `.`, `-` and `_` only.
    pub identifier: String,
    pub display_name: String,
    pub kind: ControlKind,
    /// The value a toggle starts with. Buttons have no value and ignore it.
    pub initial_value: bool,
}

/// Why the registry refused a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    EmptyIdentifier,
    IdentifierTooLong { length: usize },
    InvalidIdentifier { identifier: String },
    EmptyDisplayName,
    Duplicate { identifier: String },
    Unknown { identifier: String },
    /// The operation does not apply to this kind of control.
    WrongKind { identifier: String, kind: ControlKind },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    configuration: StaticControlConfiguration,
    value: bool,
}

/// The controls one app has registered, in the order it registered them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlRegistry {
    entries: Vec<Entry>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a control. The identifier must be valid and unused, and the display name must not be blank.
    pub fn register(&mut self, configuration: StaticControlConfiguration) -> Result<(), ControlError> {
        validate_identifier(&configuration.identifier)?;
        if configuration.display_name.trim().is_empty() {
            return Err(ControlError::EmptyDisplayName);
        }
        if self.index_of(&configuration.identifier).is_some() {
            return Err(ControlError::Duplicate { identifier: configuration.identifier });
        }
        let value = configuration.kind == ControlKind::Toggle && configuration.initial_value;
        self.entries.push(Entry { configuration, value });
        Ok(())
    }

    /// Removes a control. Returns false when no control has the identifier.
    pub fn unregister(&mut self, identifier: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.configuration.identifier != identifier);
        self.entries.len() != before
    }

    /// The registered configurations, in registration order.
    pub fn configurations(&self) -> Vec<&StaticControlConfiguration> {
        self.entries.iter().map(|e| &e.configuration).collect()
    }

    /// The value of a toggle.
    pub fn value(&self, identifier: &str) -> Result<bool, ControlError> {
        Ok(self.toggle_entry(identifier)?.value)
    }

    /// Sets the value of a toggle.
    pub fn set_value(&mut self, identifier: &str, on: bool) -> Result<(), ControlError> {
        self.toggle_entry_mut(identifier)?.value = on;
        Ok(())
    }

    /// Flips a toggle and returns its new value.
    pub fn toggle(&mut self, identifier: &str) -> Result<bool, ControlError> {
        let entry = self.toggle_entry_mut(identifier)?;
        entry.value = !entry.value;
        Ok(entry.value)
    }

    /// Records a press of a button. The action itself runs in the app, which receives the press from the shell.
    pub fn press(&self, identifier: &str) -> Result<(), ControlError> {
        let index = self.entry_index(identifier)?;
        match self.entries[index].configuration.kind {
            ControlKind::Button => Ok(()),
            kind => Err(ControlError::WrongKind { identifier: identifier.into(), kind }),
        }
    }

    fn index_of(&self, identifier: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.configuration.identifier == identifier)
    }

    fn entry_index(&self, identifier: &str) -> Result<usize, ControlError> {
        self.index_of(identifier).ok_or_else(|| ControlError::Unknown { identifier: identifier.into() })
    }

    fn toggle_entry(&self, identifier: &str) -> Result<&Entry, ControlError> {
        let index = self.toggle_index(identifier)?;
        Ok(&self.entries[index])
    }

    fn toggle_entry_mut(&mut self, identifier: &str) -> Result<&mut Entry, ControlError> {
        let index = self.toggle_index(identifier)?;
        Ok(&mut self.entries[index])
    }

    fn toggle_index(&self, identifier: &str) -> Result<usize, ControlError> {
        let index = self.entry_index(identifier)?;
        match self.entries[index].configuration.kind {
            ControlKind::Toggle => Ok(index),
            kind => Err(ControlError::WrongKind { identifier: identifier.into(), kind }),
        }
    }
}

fn validate_identifier(identifier: &str) -> Result<(), ControlError> {
    if identifier.is_empty() {
        return Err(ControlError::EmptyIdentifier);
    }
    if identifier.len() > MAX_IDENTIFIER_LEN {
        return Err(ControlError::IdentifierTooLong { length: identifier.len() });
    }
    if !identifier.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
        return Err(ControlError::InvalidIdentifier { identifier: identifier.into() });
    }
    Ok(())
}
