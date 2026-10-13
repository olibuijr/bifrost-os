//! The default calling app: which installed apps can make and take calls, and which one the system uses for
//! calls the user does not start from an app. An app registers itself when it is set up for calling (its
//! "preparing for calling" step). The user picks the default from the apps that registered. Unregistering an app
//! clears the default when it was the default. Nothing here starts a call; the call controller in `call.rs` does.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppError {
    /// The app id is empty or only spaces.
    Empty,
    /// The app did not register for calling, so it cannot be the default.
    NotRegistered(String),
}

/// The registered calling apps, in registration order, and the chosen default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallingApps {
    apps: Vec<String>,
    default: Option<String>,
}

impl CallingApps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an app for calling. Registering an app twice changes nothing.
    pub fn register(&mut self, app_id: &str) -> Result<(), AppError> {
        if app_id.trim().is_empty() {
            return Err(AppError::Empty);
        }
        if !self.is_registered(app_id) {
            self.apps.push(app_id.to_string());
        }
        Ok(())
    }

    /// Removes an app from the calling apps. Returns false when it was not registered. If the app was the default,
    /// the default is cleared.
    pub fn unregister(&mut self, app_id: &str) -> bool {
        let before = self.apps.len();
        self.apps.retain(|id| id != app_id);
        if self.default.as_deref() == Some(app_id) {
            self.default = None;
        }
        self.apps.len() != before
    }

    /// Chooses the default calling app. Only a registered app can be chosen.
    pub fn set_default(&mut self, app_id: &str) -> Result<(), AppError> {
        if app_id.trim().is_empty() {
            return Err(AppError::Empty);
        }
        if !self.is_registered(app_id) {
            return Err(AppError::NotRegistered(app_id.to_string()));
        }
        self.default = Some(app_id.to_string());
        Ok(())
    }

    /// Clears the default. The system then takes calls with no app.
    pub fn clear_default(&mut self) {
        self.default = None;
    }

    pub fn default_app(&self) -> Option<&str> {
        self.default.as_deref()
    }

    pub fn apps(&self) -> &[String] {
        &self.apps
    }

    pub fn is_registered(&self, app_id: &str) -> bool {
        self.apps.iter().any(|id| id == app_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_keeps_order_and_ignores_repeats() {
        let mut apps = CallingApps::new();
        apps.register("phone").unwrap();
        apps.register("voip").unwrap();
        apps.register("phone").unwrap();
        assert_eq!(apps.apps(), &["phone".to_string(), "voip".to_string()]);
        assert_eq!(apps.register("  "), Err(AppError::Empty));
    }

    #[test]
    fn only_a_registered_app_can_be_the_default() {
        let mut apps = CallingApps::new();
        assert_eq!(apps.set_default("voip"), Err(AppError::NotRegistered("voip".into())));
        assert_eq!(apps.default_app(), None);

        apps.register("voip").unwrap();
        apps.set_default("voip").unwrap();
        assert_eq!(apps.default_app(), Some("voip"));
        apps.clear_default();
        assert_eq!(apps.default_app(), None);
        assert_eq!(apps.set_default(""), Err(AppError::Empty));
    }

    #[test]
    fn unregistering_the_default_clears_it() {
        let mut apps = CallingApps::new();
        apps.register("phone").unwrap();
        apps.register("voip").unwrap();
        apps.set_default("voip").unwrap();

        assert!(!apps.unregister("missing"));
        assert_eq!(apps.default_app(), Some("voip"));

        assert!(apps.unregister("voip"));
        assert_eq!(apps.default_app(), None);
        assert!(!apps.is_registered("voip"));
        assert!(apps.is_registered("phone"));
    }
}
