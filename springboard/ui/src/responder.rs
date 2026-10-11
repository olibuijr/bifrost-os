//! The responder chain (`NativeResponder`, `NativeResponder`): an action goes to the first responder and travels outwards
//! until someone handles it. Menu items and toolbar buttons are enabled by the same walk (`validateMenuItem:`).

/// One link in the chain: a focused view, then its container, window and app.
pub trait Responder {
    /// Handles `action` if this responder implements it; true when handled (the walk stops).
    fn perform(&mut self, _action: &str) -> bool {
        false
    }
    /// `Some(enabled)` when this responder knows the action, `None` to defer to the next link.
    fn validate(&self, _action: &str) -> Option<bool> {
        None
    }
}

/// Responders from the first responder outwards.
#[derive(Default)]
pub struct ResponderChain<'a> {
    links: Vec<&'a mut dyn Responder>,
}

impl<'a> ResponderChain<'a> {
    pub fn new() -> Self {
        ResponderChain { links: Vec::new() }
    }
    /// Adds the next link outwards (call first for the first responder).
    pub fn push(mut self, r: &'a mut dyn Responder) -> Self {
        self.links.push(r);
        self
    }
    pub fn len(&self) -> usize {
        self.links.len()
    }
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }
    /// Sends `action` along the chain; returns the index of the responder that handled it.
    pub fn send(&mut self, action: &str) -> Option<usize> {
        self.links.iter_mut().position(|r| r.perform(action))
    }
    /// Whether a menu item for `action` is enabled: the first responder with an opinion decides, and an action nobody
    /// knows is disabled.
    pub fn is_enabled(&self, action: &str) -> bool {
        self.links.iter().find_map(|r| r.validate(action)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Field {
        text: String,
        selection: bool,
    }
    struct Window {
        closed: bool,
    }
    struct App {
        log: Vec<String>,
    }
    impl Responder for Field {
        fn perform(&mut self, a: &str) -> bool {
            match a {
                "copy" if self.selection => {
                    self.text.push('!');
                    true
                }
                "selectAll" => {
                    self.selection = true;
                    true
                }
                _ => false,
            }
        }
        fn validate(&self, a: &str) -> Option<bool> {
            match a {
                "copy" | "cut" => Some(self.selection),
                "selectAll" | "paste" => Some(true),
                _ => None,
            }
        }
    }
    impl Responder for Window {
        fn perform(&mut self, a: &str) -> bool {
            if a == "close" {
                self.closed = true;
            }
            a == "close"
        }
        fn validate(&self, a: &str) -> Option<bool> {
            (a == "close").then_some(!self.closed)
        }
    }
    impl Responder for App {
        fn perform(&mut self, a: &str) -> bool {
            self.log.push(a.to_string());
            a == "quit"
        }
        fn validate(&self, a: &str) -> Option<bool> {
            (a == "quit").then_some(true)
        }
    }

    #[test]
    fn an_action_goes_to_the_first_responder_that_handles_it() {
        let (mut f, mut w, mut a) = (Field { text: String::new(), selection: false }, Window { closed: false }, App { log: vec![] });
        let mut chain = ResponderChain::new().push(&mut f).push(&mut w).push(&mut a);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.send("selectAll"), Some(0));
        assert_eq!(chain.send("close"), Some(1), "the field ignores it, so the window handles it");
        assert_eq!(chain.send("quit"), Some(2));
        assert_eq!(chain.send("copy"), Some(0), "selection now exists");
        assert_eq!(chain.send("frobnicate"), None);
        drop(chain);
        assert_eq!(a.log, vec!["quit", "frobnicate"].iter().map(|s| s.to_string()).collect::<Vec<_>>(), "the app only saw what the others passed up");
        assert!(w.closed && f.text == "!");
    }

    #[test]
    fn menu_validation_asks_the_chain_and_unknown_actions_are_disabled() {
        let (mut f, mut w) = (Field { text: String::new(), selection: false }, Window { closed: false });
        {
            let chain = ResponderChain::new().push(&mut f).push(&mut w);
            assert!(!chain.is_enabled("copy"), "no selection: Copy is dimmed");
            assert!(chain.is_enabled("paste"));
            assert!(chain.is_enabled("close"), "falls through to the window");
            assert!(!chain.is_enabled("zoom"), "nobody implements it");
        }
        f.selection = true;
        assert!(ResponderChain::new().push(&mut f).is_enabled("copy"));
        assert!(ResponderChain::new().is_empty());
    }
}
