//! Notification content changes before delivery: a chain of steps that each edit a copy of a notification's content,
//! run within a time budget. A step that fails drops its own edits and the content before it is delivered. When the
//! budget runs out before a step starts, the content so far is delivered, the same fallback as the system's "time will
//! expire" call: an unfinished extension shows what it finished, never nothing. The model does no I/O; the shell
//! supplies the clock and calls [`Extension::deliver`] before it shows a notification.

/// The content of a notification as a step sees it: the fields of the content an app can change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub thread: String,
    pub category: String,
    pub badge: Option<u32>,
    /// custom data (`userInfo`), in insertion order
    pub user_info: Vec<(String, String)>,
}

/// One step: edits the content, or returns a reason why it gave up.
pub type Step = Box<dyn FnMut(&mut Content) -> Result<(), &'static str>>;

/// How a delivery ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// every step finished in time
    Complete,
    /// the step at this index failed; the content before it is delivered and later steps do not run
    Failed { step: usize, reason: &'static str },
    /// the budget ran out before the step at this index started; the content so far is delivered
    Expired { step: usize },
}

/// The result of [`Extension::deliver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub content: Content,
    pub outcome: Outcome,
}

/// A notification service extension: an ordered list of steps and the time they may take together.
pub struct Extension {
    steps: Vec<Step>,
    budget_ms: u32,
}

impl Extension {
    /// An extension with no steps that may spend `budget_ms` milliseconds before it must deliver.
    pub fn new(budget_ms: u32) -> Self {
        Extension { steps: Vec::new(), budget_ms }
    }

    /// Appends a step. Steps run in the order they were added.
    pub fn step(mut self, step: impl FnMut(&mut Content) -> Result<(), &'static str> + 'static) -> Self {
        self.steps.push(Box::new(step));
        self
    }

    /// Runs the steps on a copy of `original`; the original is not changed. `clock` returns the milliseconds spent so
    /// far and is read before each step. Once it reaches the budget, the next step does not run.
    pub fn deliver(&mut self, original: &Content, mut clock: impl FnMut() -> u32) -> Delivery {
        let mut content = original.clone();
        for (i, step) in self.steps.iter_mut().enumerate() {
            if clock() >= self.budget_ms {
                return Delivery { content, outcome: Outcome::Expired { step: i } };
            }
            let mut next = content.clone();
            match step(&mut next) {
                Ok(()) => content = next,
                Err(reason) => return Delivery { content, outcome: Outcome::Failed { step: i, reason } },
            }
        }
        Delivery { content, outcome: Outcome::Complete }
    }
}

/// A step that shortens the body to at most `max` characters, the last one being an ellipsis. A body that already fits
/// is left alone. `max` of 0 has no room for any text and fails.
pub fn truncate_body(max: usize) -> impl FnMut(&mut Content) -> Result<(), &'static str> {
    move |c: &mut Content| {
        if max == 0 {
            return Err("no room for the body");
        }
        if c.body.chars().count() > max {
            let kept: String = c.body.chars().take(max - 1).collect();
            c.body = kept + "…";
        }
        Ok(())
    }
}

/// A step that sets `key` in the custom data, replacing an earlier value for the same key.
pub fn set_info(key: &str, value: &str) -> impl FnMut(&mut Content) -> Result<(), &'static str> + 'static {
    let key = key.to_string();
    let value = value.to_string();
    move |c: &mut Content| {
        match c.user_info.iter().position(|(k, _)| *k == key) {
            Some(i) => c.user_info[i].1 = value.clone(),
            None => c.user_info.push((key.clone(), value.clone())),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Content {
        Content { title: "T".into(), body: "hello world".into(), ..Default::default() }
    }

    #[test]
    fn steps_run_in_order_and_each_sees_the_last_edit() {
        let mut ext = Extension::new(1000)
            .step(|c: &mut Content| {
                c.title = "A".into();
                Ok(())
            })
            .step(|c: &mut Content| {
                c.title.push('!');
                Ok(())
            });
        let d = ext.deliver(&base(), || 0);
        assert_eq!(d.outcome, Outcome::Complete);
        assert_eq!(d.content.title, "A!");
        assert_eq!(d.content.body, "hello world");
    }

    #[test]
    fn failing_step_drops_its_own_edits_and_stops_the_chain() {
        let mut ext = Extension::new(1000)
            .step(|c: &mut Content| {
                c.body = "x".into();
                Ok(())
            })
            .step(|c: &mut Content| {
                c.title = "bad".into();
                Err("no")
            })
            .step(|c: &mut Content| {
                c.category = "never".into();
                Ok(())
            });
        let d = ext.deliver(&base(), || 0);
        assert_eq!(d.outcome, Outcome::Failed { step: 1, reason: "no" });
        assert_eq!(d.content.body, "x");
        assert_eq!(d.content.title, "T");
        assert_eq!(d.content.category, "");
    }

    #[test]
    fn budget_expiry_delivers_the_steps_that_finished() {
        let mut ext = Extension::new(100)
            .step(|c: &mut Content| {
                c.badge = Some(1);
                Ok(())
            })
            .step(|c: &mut Content| {
                c.badge = Some(2);
                Ok(())
            })
            .step(|c: &mut Content| {
                c.badge = Some(3);
                Ok(())
            });
        // the clock is read before each step: 0 and 60 are under the budget of 100, 120 is over it
        let mut ticks = vec![0u32, 60, 120].into_iter();
        let d = ext.deliver(&base(), || ticks.next().unwrap());
        assert_eq!(d.outcome, Outcome::Expired { step: 2 });
        assert_eq!(d.content.badge, Some(2));
    }

    #[test]
    fn zero_budget_runs_nothing_and_delivers_the_original() {
        let mut ext = Extension::new(0).step(|c: &mut Content| {
            c.title = "changed".into();
            Ok(())
        });
        let original = base();
        let d = ext.deliver(&original, || 0);
        assert_eq!(d.outcome, Outcome::Expired { step: 0 });
        assert_eq!(d.content, original);
    }

    #[test]
    fn empty_chain_completes_with_the_original_and_the_original_is_not_changed() {
        let mut ext = Extension::new(10);
        let original = base();
        let d = ext.deliver(&original, || 0);
        assert_eq!(d.outcome, Outcome::Complete);
        assert_eq!(d.content, original);
    }

    #[test]
    fn truncate_body_keeps_max_characters_including_the_ellipsis() {
        let mut f = truncate_body(6);
        let mut c = base();
        assert_eq!(f(&mut c), Ok(()));
        assert_eq!(c.body, "hello…");
        assert_eq!(c.body.chars().count(), 6);

        let mut short = Content { body: "short".into(), ..Default::default() };
        assert_eq!(f(&mut short), Ok(()));
        assert_eq!(short.body, "short");

        let mut none = truncate_body(0);
        assert_eq!(none(&mut short), Err("no room for the body"));
    }

    #[test]
    fn set_info_replaces_an_existing_key_and_appends_a_new_one() {
        let mut ext = Extension::new(1000)
            .step(set_info("chat", "1"))
            .step(set_info("n", "2"))
            .step(set_info("chat", "3"));
        let d = ext.deliver(&base(), || 0);
        assert_eq!(d.content.user_info, vec![("chat".to_string(), "3".to_string()), ("n".to_string(), "2".to_string())]);
    }
}
