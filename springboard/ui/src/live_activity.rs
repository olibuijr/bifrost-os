//! Live activities: a short status an app starts and keeps up to date, shown by the shell for as long as it is useful
//! (a lock screen strip, a compact tile). This is the life cycle only: a request, which may be scheduled for later,
//! updates while the activity is active or stale, and an end with a dismissal policy. The state of an activity at a
//! given moment is derived from its times, so nothing needs a timer. Times are whole seconds since the epoch and are
//! passed in by the caller; nothing here reads a clock. Presentation (which regions show what) belongs to the shell.

/// The longest an ended activity may stay on screen: four hours, in seconds.
pub const DISMISSAL_WINDOW_S: u64 = 4 * 60 * 60;

/// Where an activity is in its life cycle at a given time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Scheduled to start at a later time.
    Pending,
    /// Visible and open to content updates.
    Active,
    /// Visible, but its content is past its stale date and needs an update.
    Stale,
    /// Visible, but ended: no more content updates.
    Ended,
    /// Ended and removed from the screen.
    Dismissed,
}

/// The content of an activity: the app's own payload and the time after which that payload is out of date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content<T> {
    pub state: T,
    pub stale_date: Option<u64>,
}

impl<T> Content<T> {
    pub fn new(state: T) -> Self {
        Content { state, stale_date: None }
    }

    /// The same content, stale from `t` on.
    pub fn stale_at(mut self, t: u64) -> Self {
        self.stale_date = Some(t);
        self
    }
}

/// When the shell removes an activity after it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissal {
    /// The shell's own choice. BifrOSt keeps an ended activity for the whole [`DISMISSAL_WINDOW_S`].
    Default,
    /// Removed at the moment it ends.
    Immediate,
    /// Removed at this time. A time already past means immediately, and a later time is cut back to the window.
    After(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The person has turned live activities off.
    Disabled,
    /// No activity has this id.
    NotFound,
    /// The activity is not in a state that accepts this call.
    NotLive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Activity<T> {
    id: u32,
    app: String,
    content: Content<T>,
    start: u64,
    /// When it ended, and when it is removed.
    ended: Option<(u64, u64)>,
}

/// The activities of every app on the desktop, and the person's switch for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry<T> {
    enabled: bool,
    next_id: u32,
    activities: Vec<Activity<T>>,
}

/// The removal time for an activity that ends at `ended_at`.
fn removal_time(dismissal: Dismissal, ended_at: u64) -> u64 {
    let latest = ended_at + DISMISSAL_WINDOW_S;
    match dismissal {
        Dismissal::Immediate => ended_at,
        Dismissal::Default => latest,
        Dismissal::After(t) => t.max(ended_at).min(latest),
    }
}

impl<T: Clone> Registry<T> {
    pub fn new(enabled: bool) -> Self {
        Registry { enabled, next_id: 1, activities: Vec::new() }
    }

    /// The person's switch. Turning it off stops new requests; running activities keep their state.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Starts an activity for `app` and returns its id. Without a start time it starts at `now`; a start time after
    /// `now` makes it pending until then.
    pub fn request(
        &mut self,
        app: &str,
        content: Content<T>,
        start: Option<u64>,
        now: u64,
    ) -> Result<u32, Error> {
        if !self.enabled {
            return Err(Error::Disabled);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.activities.push(Activity {
            id,
            app: app.into(),
            content,
            start: start.unwrap_or(now),
            ended: None,
        });
        Ok(id)
    }

    /// The state of an activity at `now`, or `None` for an unknown id.
    pub fn state(&self, id: u32, now: u64) -> Option<State> {
        let a = self.activities.iter().find(|a| a.id == id)?;
        // before its end time an ended activity reports the state it had then
        Some(match a.ended {
            Some((_, removed)) if now >= removed => State::Dismissed,
            Some((ended_at, _)) if now >= ended_at => State::Ended,
            _ if now < a.start => State::Pending,
            _ if a.content.stale_date.map_or(false, |s| now >= s) => State::Stale,
            _ => State::Active,
        })
    }

    /// The content an activity shows at `now`. `None` for an unknown or dismissed activity.
    pub fn content(&self, id: u32, now: u64) -> Option<&Content<T>> {
        match self.state(id, now)? {
            State::Dismissed => None,
            _ => self.activities.iter().find(|a| a.id == id).map(|a| &a.content),
        }
    }

    /// Replaces the content of an active or stale activity.
    pub fn update(&mut self, id: u32, content: Content<T>, now: u64) -> Result<(), Error> {
        match self.state(id, now).ok_or(Error::NotFound)? {
            State::Active | State::Stale => {}
            _ => return Err(Error::NotLive),
        }
        let a = self.activities.iter_mut().find(|a| a.id == id).ok_or(Error::NotFound)?;
        a.content = content;
        Ok(())
    }

    /// Ends an active or stale activity. A final content, if given, replaces the content first. The activity stays
    /// on screen as ended until `dismissal` allows its removal.
    pub fn end(
        &mut self,
        id: u32,
        final_content: Option<Content<T>>,
        dismissal: Dismissal,
        now: u64,
    ) -> Result<(), Error> {
        match self.state(id, now).ok_or(Error::NotFound)? {
            State::Active | State::Stale => {}
            _ => return Err(Error::NotLive),
        }
        let a = self.activities.iter_mut().find(|a| a.id == id).ok_or(Error::NotFound)?;
        if let Some(c) = final_content {
            a.content = c;
        }
        a.ended = Some((now, removal_time(dismissal, now)));
        Ok(())
    }

    /// The ids of an app's activities that are on screen at `now` (not dismissed), in request order.
    pub fn visible(&self, app: &str, now: u64) -> Vec<u32> {
        self.activities
            .iter()
            .filter(|a| a.app == app)
            .filter(|a| self.state(a.id, now).map_or(false, |s| s != State::Dismissed))
            .map(|a| a.id)
            .collect()
    }

    /// The ids for the lock screen strip at `now`: live activities (active or stale) of every app first, then ended
    /// ones still on screen, each group in request order, at most `limit`. Pending and dismissed activities are left out.
    pub fn strip(&self, now: u64, limit: usize) -> Vec<u32> {
        let mut live = Vec::new();
        let mut ended = Vec::new();
        for a in &self.activities {
            match self.state(a.id, now) {
                Some(State::Active | State::Stale) => live.push(a.id),
                Some(State::Ended) => ended.push(a.id),
                _ => {}
            }
        }
        live.into_iter().chain(ended).take(limit).collect()
    }}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_refused_while_the_person_has_turned_activities_off() {
        let mut r: Registry<&'static str> = Registry::new(false);
        assert_eq!(r.request("app.a", Content::new("a"), None, 100), Err(Error::Disabled));
        r.set_enabled(true);
        let id = r.request("app.a", Content::new("a"), None, 100).unwrap();
        assert_eq!(id, 1);
        assert_eq!(r.state(1, 100), Some(State::Active));
    }

    #[test]
    fn a_scheduled_activity_is_pending_until_its_start_time() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("s"), Some(160), 100).unwrap();
        assert_eq!(r.state(1, 100), Some(State::Pending));
        assert_eq!(r.state(1, 159), Some(State::Pending));
        assert_eq!(r.state(1, 160), Some(State::Active));
    }

    #[test]
    fn content_goes_stale_at_its_stale_date_and_an_update_clears_it() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("s").stale_at(200), None, 100).unwrap();
        assert_eq!(r.state(1, 199), Some(State::Active));
        assert_eq!(r.state(1, 200), Some(State::Stale));
        r.update(1, Content::new("t").stale_at(300), 200).unwrap();
        assert_eq!(r.state(1, 200), Some(State::Active));
        assert_eq!(r.state(1, 300), Some(State::Stale));
        assert_eq!(r.content(1, 200).map(|c| c.state), Some("t"));
    }

    #[test]
    fn an_immediate_end_removes_the_activity_at_once() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("a"), None, 100).unwrap();
        r.end(1, None, Dismissal::Immediate, 500).unwrap();
        assert_eq!(r.state(1, 499), Some(State::Active));
        assert_eq!(r.state(1, 500), Some(State::Dismissed));
        assert_eq!(r.content(1, 500), None);
        assert!(r.visible("app.a", 500).is_empty());
    }

    #[test]
    fn an_after_dismissal_is_cut_back_to_the_four_hour_window() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("a"), None, 100).unwrap();
        // Asked for 10 hours after the end; the window allows 4 hours, so removal is at 1000 + 14400 = 15400.
        r.end(1, None, Dismissal::After(1000 + 10 * 3600), 1000).unwrap();
        assert_eq!(r.state(1, 15399), Some(State::Ended));
        assert_eq!(r.state(1, 15400), Some(State::Dismissed));
    }

    #[test]
    fn a_past_removal_time_removes_at_once_and_default_keeps_the_window() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("a"), None, 100).unwrap();
        r.request("app.a", Content::new("b"), None, 100).unwrap();
        r.end(1, None, Dismissal::After(500), 1000).unwrap();
        r.end(2, None, Dismissal::Default, 1000).unwrap();
        assert_eq!(r.state(1, 1000), Some(State::Dismissed));
        assert_eq!(r.state(2, 1000), Some(State::Ended));
        assert_eq!(r.state(2, 1000 + DISMISSAL_WINDOW_S - 1), Some(State::Ended));
        assert_eq!(r.state(2, 1000 + DISMISSAL_WINDOW_S), Some(State::Dismissed));
    }

    #[test]
    fn a_final_content_replaces_the_content_when_the_activity_ends() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("a"), None, 100).unwrap();
        r.end(1, Some(Content::new("done")), Dismissal::Default, 200).unwrap();
        assert_eq!(r.state(1, 200), Some(State::Ended));
        assert_eq!(r.content(1, 200), Some(&Content::new("done")));
    }

    #[test]
    fn a_pending_or_ended_activity_refuses_updates_and_ends_and_unknown_ids_are_not_found() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("x"), Some(200), 100).unwrap();
        assert_eq!(r.update(1, Content::new("y"), 100), Err(Error::NotLive));
        assert_eq!(r.end(1, None, Dismissal::Immediate, 100), Err(Error::NotLive));
        assert_eq!(r.update(9, Content::new("y"), 100), Err(Error::NotFound));
        r.request("app.a", Content::new("z"), None, 300).unwrap();
        r.end(2, None, Dismissal::Default, 300).unwrap();
        assert_eq!(r.end(2, None, Dismissal::Default, 301), Err(Error::NotLive));
        assert_eq!(r.update(2, Content::new("w"), 301), Err(Error::NotLive));
    }

    #[test]
    fn visible_lists_only_the_apps_own_activities_that_are_not_dismissed() {
        let mut r: Registry<&'static str> = Registry::new(true);
        r.request("app.a", Content::new("a"), None, 100).unwrap();
        r.request("app.b", Content::new("b"), None, 100).unwrap();
        r.request("app.a", Content::new("c"), None, 100).unwrap();
        r.end(3, None, Dismissal::Immediate, 150).unwrap();
        assert_eq!(r.visible("app.a", 150), vec![1]);
        assert_eq!(r.visible("app.b", 150), vec![2]);
    }

    #[test]
    fn the_strip_lists_live_activities_before_ended_ones_and_skips_pending_and_dismissed() {
        let mut r: Registry<u8> = Registry::new(true);
        let ended = r.request("a", Content::new(1), None, 100).unwrap();
        let later = r.request("b", Content::new(2), Some(500), 100).unwrap();
        let live = r.request("c", Content::new(3), None, 100).unwrap();
        r.end(ended, None, Dismissal::Default, 150).unwrap();
        let gone = r.request("d", Content::new(4), None, 100).unwrap();
        r.end(gone, None, Dismissal::Immediate, 160).unwrap();
        assert_eq!(r.strip(200, 4), [live, ended], "pending and dismissed activities are not on the strip");
        assert_eq!(r.strip(200, 1), [live], "the limit caps the strip");
        assert_eq!(r.strip(600, 4), [later, live, ended], "a started activity joins the live group in request order");
    }
}
