//! User activities: what an app is doing right now, so that another app can continue it (handoff) and a search can
//! find it. The model is pure and holds no I/O. The shell keeps one `ActivityCenter` for the session. An owner makes
//! an activity current, adds the state it needs to resume (`user_info`), and invalidates it when the work ends or the
//! owner quits. Making a new activity of a type current replaces the owner's previous one of that type.
use std::collections::BTreeMap;

/// One activity an app has made current.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserActivity {
    pub id: u64,
    /// The app that owns the activity.
    pub owner: String,
    /// A reverse-DNS style type name, such as `com.example.app.editing`.
    pub activity_type: String,
    pub title: String,
    /// The state another app needs to resume the work.
    pub user_info: BTreeMap<String, String>,
    pub eligible_for_handoff: bool,
    pub eligible_for_search: bool,
}

/// The session's activities. Ids increase, so the highest id is the newest.
#[derive(Clone, Debug, Default)]
pub struct ActivityCenter {
    next_id: u64,
    activities: Vec<UserActivity>,
}

impl ActivityCenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes a new activity current for `owner`, replacing the owner's previous activity of the same type. Handoff is
    /// on by default. None when the owner or the type is empty.
    pub fn become_current(&mut self, owner: &str, activity_type: &str, title: &str) -> Option<u64> {
        if owner.is_empty() || activity_type.is_empty() {
            return None;
        }
        self.activities.retain(|a| !(a.owner == owner && a.activity_type == activity_type));
        self.next_id += 1;
        let id = self.next_id;
        self.activities.push(UserActivity {
            id,
            owner: owner.into(),
            activity_type: activity_type.into(),
            title: title.into(),
            user_info: BTreeMap::new(),
            eligible_for_handoff: true,
            eligible_for_search: false,
        });
        Some(id)
    }

    /// The activity with this id, if it is still current.
    pub fn activity(&self, id: u64) -> Option<&UserActivity> {
        self.activities.iter().find(|a| a.id == id)
    }

    /// Stores one key of resume state on an activity. False when the activity is not current.
    pub fn set_user_info(&mut self, id: u64, key: &str, value: &str) -> bool {
        match self.activities.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.user_info.insert(key.into(), value.into());
                true
            }
            None => false,
        }
    }

    /// Sets whether an activity may be handed off to another app and whether search may list it. False when the
    /// activity is not current.
    pub fn set_eligibility(&mut self, id: u64, handoff: bool, search: bool) -> bool {
        match self.activities.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.eligible_for_handoff = handoff;
                a.eligible_for_search = search;
                true
            }
            None => false,
        }
    }

    /// Ends one activity. False when it was not current.
    pub fn invalidate(&mut self, id: u64) -> bool {
        let before = self.activities.len();
        self.activities.retain(|a| a.id != id);
        self.activities.len() != before
    }

    /// Ends every activity of an owner, for example when the app quits. Returns how many ended.
    pub fn invalidate_owner(&mut self, owner: &str) -> usize {
        let before = self.activities.len();
        self.activities.retain(|a| a.owner != owner);
        before - self.activities.len()
    }

    /// The newest handoff-eligible activity of `activity_type` owned by an app other than `requester`: what that app
    /// would continue.
    pub fn continue_activity(&self, requester: &str, activity_type: &str) -> Option<&UserActivity> {
        self.activities
            .iter()
            .filter(|a| a.eligible_for_handoff && a.owner != requester && a.activity_type == activity_type)
            .max_by_key(|a| a.id)
    }

    /// Every current activity that search may list, newest first.
    pub fn searchable(&self) -> Vec<&UserActivity> {
        let mut found: Vec<&UserActivity> = self.activities.iter().filter(|a| a.eligible_for_search).collect();
        found.sort_by(|a, b| b.id.cmp(&a.id));
        found
    }

    pub fn len(&self) -> usize {
        self.activities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.activities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn becoming_current_returns_an_id_and_refuses_empty_owner_or_type() {
        let mut c = ActivityCenter::new();
        assert_eq!(c.become_current("", "x.edit", "Doc"), None);
        assert_eq!(c.become_current("notes", "", "Doc"), None);
        let id = c.become_current("notes", "x.edit", "Doc").unwrap();
        assert_eq!(c.activity(id).map(|a| a.title.as_str()), Some("Doc"));
        assert!(c.activity(id).unwrap().eligible_for_handoff);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn a_new_activity_of_the_same_type_replaces_the_owners_previous_one() {
        let mut c = ActivityCenter::new();
        let first = c.become_current("notes", "x.edit", "A").unwrap();
        let second = c.become_current("notes", "x.edit", "B").unwrap();
        assert!(c.activity(first).is_none());
        assert_eq!(c.activity(second).map(|a| a.title.as_str()), Some("B"));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn different_types_and_owners_coexist() {
        let mut c = ActivityCenter::new();
        c.become_current("notes", "x.edit", "A").unwrap();
        c.become_current("notes", "x.read", "B").unwrap();
        c.become_current("mail", "x.edit", "C").unwrap();
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn another_app_continues_the_newest_eligible_activity_of_the_type() {
        let mut c = ActivityCenter::new();
        c.become_current("notes", "x.edit", "Old").unwrap();
        let newest = c.become_current("mail", "x.edit", "New").unwrap();
        assert_eq!(c.continue_activity("files", "x.edit").map(|a| a.id), Some(newest));
        // The owner does not continue its own activity.
        assert_eq!(c.continue_activity("mail", "x.edit").map(|a| a.title.as_str()), Some("Old"));
        assert!(c.continue_activity("files", "x.other").is_none());
    }

    #[test]
    fn an_activity_not_eligible_for_handoff_is_not_continued() {
        let mut c = ActivityCenter::new();
        let id = c.become_current("notes", "x.edit", "A").unwrap();
        assert!(c.set_eligibility(id, false, true));
        assert!(c.continue_activity("files", "x.edit").is_none());
        assert_eq!(c.searchable().iter().map(|a| a.id).collect::<Vec<_>>(), vec![id]);
    }

    #[test]
    fn user_info_is_kept_on_the_activity() {
        let mut c = ActivityCenter::new();
        let id = c.become_current("notes", "x.edit", "A").unwrap();
        assert!(c.set_user_info(id, "cursor", "42"));
        assert!(c.set_user_info(id, "cursor", "43"));
        assert_eq!(c.activity(id).unwrap().user_info.get("cursor").map(String::as_str), Some("43"));
        assert!(!c.set_user_info(999, "cursor", "1"));
    }

    #[test]
    fn invalidate_and_invalidate_owner_end_activities() {
        let mut c = ActivityCenter::new();
        let a = c.become_current("notes", "x.edit", "A").unwrap();
        c.become_current("notes", "x.read", "B").unwrap();
        c.become_current("mail", "x.edit", "C").unwrap();
        assert!(c.invalidate(a));
        assert!(!c.invalidate(a));
        assert_eq!(c.invalidate_owner("notes"), 1);
        assert_eq!(c.invalidate_owner("notes"), 0);
        assert_eq!(c.len(), 1);
        assert!(!c.is_empty());
    }

    #[test]
    fn search_lists_only_eligible_activities_newest_first() {
        let mut c = ActivityCenter::new();
        let a = c.become_current("notes", "x.edit", "A").unwrap();
        let b = c.become_current("mail", "x.edit", "B").unwrap();
        assert!(c.searchable().is_empty());
        c.set_eligibility(a, true, true);
        c.set_eligibility(b, true, true);
        let ids: Vec<u64> = c.searchable().iter().map(|x| x.id).collect();
        assert_eq!(ids, vec![b, a]);
    }
}
