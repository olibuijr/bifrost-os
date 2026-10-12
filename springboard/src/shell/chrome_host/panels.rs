//! The shell side of the panels client: builds the `PanelsModel` for Notification Center from the notification
//! store. Owner: `chrome-panels`.

use crate::shell::apps;
use crate::shell::notify::store::{Store, age_text};
use bifrost_chrome::model::{Notification as ChromeNotification, PanelsModel};

/// The Notification Center model: every stored notification, grouped by app on the client. The app id is the
/// app's bundle id (its `icon` key, as the store persists it); `now` is the shell's clock in Unix seconds, used for
/// the age text. Control Center modules are not produced yet (see docs/graphics/g2-chrome-panels.md).
pub fn model(store: &Store, now: i64) -> PanelsModel {
    let mut notifications = Vec::with_capacity(store.len());
    for group in store.groups() {
        let app_id = apps().iter().find(|a| a.id == group.app).map_or("", |a| a.icon).to_string();
        for n in group.items {
            notifications.push(ChromeNotification {
                id: n.id as u32,
                app_id: app_id.clone(),
                title: n.title,
                body: n.body,
                time: age_text(n.time, now),
                actions: Vec::new(),
            });
        }
    }
    PanelsModel { notifications, controls: Vec::new() }
}

