//! Spoken Content: Speak Items Under the Pointer. When the pointer rests on an element of an app's window for
//! `DELAY_MS`, the element is spoken the way the screen reader names it (`walk::speech`: kind, name, value), once; moving
//! to another element starts the wait again, and coming back to the same one after leaving it speaks it again.
//!
//! The elements are the nodes the app published (`AppHost::controls`, window-local points). The shell feeds each pointer
//! position over an app's content (`pointer_events::ext_content_at`) through `Shell::hover_speech_at` and asks once a
//! tick (`Shell::hover_speech_tick`) whether the wait is over. The pure parts (`item_at`, `utterance`, `HoverSpeech`) are
//! tested on the host. The shell's own chrome and its built-in windows are not spoken yet: they publish no node frames.

use bifrost_proto::{Node, Role};

/// How long the pointer must rest on one element before it is spoken, in milliseconds.
pub const DELAY_MS: f64 = 600.0;

/// The element under window-local point (`x`, `y`): the innermost one, which is the last drawn of those that contain the
/// point. Windows and lists are containers, so they are chosen only when no element inside them contains the point.
pub fn item_at(nodes: &[Node], x: f32, y: f32) -> Option<&Node> {
    let inside = |n: &&Node| x >= n.bounds.x && y >= n.bounds.y && x < n.bounds.x + n.bounds.w && y < n.bounds.y + n.bounds.h;
    let container = |n: &&Node| matches!(n.role, Role::Window | Role::List);
    nodes.iter().rev().filter(inside).find(|n| !container(n)).or_else(|| nodes.iter().rev().filter(inside).next())
}

/// What is spoken for a node: the screen reader's words for it. A node with no label or value says nothing.
pub fn utterance(n: &Node) -> String {
    if n.label.trim().is_empty() && n.value.trim().is_empty() {
        return String::new();
    }
    let e = bifrost_ui::ax::Element {
        address: String::new(),
        name: n.label.clone(),
        role: bifrost_ui::ax::role_name(n.role).to_string(),
        value: n.value.clone(),
        enabled: n.enabled,
        bounds: None,
        handle: String::new(),
    };
    super::walk::speech(&e)
}

/// The element the pointer rests on (app slot and node id) and since when.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverSpeech {
    over: Option<(u8, u64)>,
    since: f64,
    spoken: bool,
}

impl HoverSpeech {
    /// The pointer is over `item` (`None`: over nothing that speaks) at `t` ms. A new element restarts the wait.
    pub fn feed(&mut self, item: Option<(u8, u64)>, t: f64) {
        if item != self.over {
            self.over = item;
            self.since = t;
            self.spoken = false;
        }
    }

    /// Whether the element the pointer rests on should be spoken now (at `t` ms): once per visit, after `DELAY_MS`.
    pub fn due(&mut self, t: f64) -> Option<(u8, u64)> {
        let item = self.over?;
        if self.spoken || t - self.since < DELAY_MS {
            return None;
        }
        self.spoken = true;
        Some(item)
    }

    /// Forgets the element, for example when the setting goes off.
    pub fn reset(&mut self) {
        *self = HoverSpeech::default();
    }
}

impl crate::shell::Shell {
    /// The pointer is at screen point (`x`, `y`) at `t` ms with no button down: notes the app element under it.
    pub(in crate::shell) fn hover_speech_at(&mut self, x: f32, y: f32, t: f64) {
        if !super::speak_under_pointer() {
            return;
        }
        let pt = self.ui.pt(1.0).max(0.01);
        let item = self.ext_content_at(x, y).filter(|(_, window, _, _)| *window == 0).and_then(|(slot, _, lx, ly)| {
            let host = self.ext.get(slot as usize)?.as_ref()?;
            item_at(&host.controls, lx / pt, ly / pt).filter(|n| !utterance(n).is_empty()).map(|n| (slot, n.id))
        });
        self.hover_speech.feed(item, t);
    }

    /// Once a tick: speaks the element the pointer has rested on for `DELAY_MS`.
    pub(in crate::shell) fn hover_speech_tick(&mut self, t: f64) {
        if !super::speak_under_pointer() {
            self.hover_speech.reset();
            return;
        }
        let Some((slot, id)) = self.hover_speech.due(t) else { return };
        let text = self.ext.get(slot as usize).and_then(Option::as_ref).and_then(|h| h.controls.iter().find(|n| n.id == id)).map(utterance).unwrap_or_default();
        if text.is_empty() {
            return;
        }
        let config = self.speech_config();
        self.speech.configure(config);
        self.speech.say(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_proto::UiRect;

    fn node(id: u64, role: Role, label: &str, x: f32, y: f32, w: f32, h: f32) -> Node {
        Node { id, role, label: label.into(), value: String::new(), bounds: UiRect { x, y, w, h }, enabled: true }
    }

    #[test]
    fn the_innermost_element_under_the_point_is_chosen() {
        let nodes = vec![
            node(1, Role::Window, "Mail", 0., 0., 400., 300.),
            node(2, Role::List, "Inbox", 0., 0., 200., 300.),
            node(3, Role::Row, "Invoice", 0., 40., 200., 20.),
            node(4, Role::Button, "Send", 300., 10., 60., 20.),
        ];
        assert_eq!(item_at(&nodes, 10., 50.).map(|n| n.id), Some(3));
        assert_eq!(item_at(&nodes, 310., 15.).map(|n| n.id), Some(4));
        assert_eq!(item_at(&nodes, 10., 100.).map(|n| n.id), Some(2), "a list where no row is");
        assert_eq!(item_at(&nodes, 300., 200.).map(|n| n.id), Some(1), "the window where nothing else is");
        assert_eq!(item_at(&nodes, 500., 10.), None);
    }

    #[test]
    fn an_element_is_spoken_with_the_screen_readers_words() {
        assert_eq!(utterance(&node(4, Role::Button, "Send", 0., 0., 1., 1.)), "button, Send");
        let mut off = node(5, Role::Toggle, "Bold", 0., 0., 1., 1.);
        off.value = "off".into();
        off.enabled = false;
        assert_eq!(utterance(&off), "switch, Bold, off, unavailable");
        assert_eq!(utterance(&node(6, Role::StaticText, " ", 0., 0., 1., 1.)), "");
    }

    #[test]
    fn a_resting_pointer_speaks_once_after_the_delay() {
        let mut h = HoverSpeech::default();
        h.feed(Some((1, 3)), 0.0);
        assert_eq!(h.due(DELAY_MS - 1.0), None, "not yet");
        h.feed(Some((1, 3)), DELAY_MS - 1.0);
        assert_eq!(h.due(DELAY_MS), Some((1, 3)), "moving inside the element keeps the wait");
        assert_eq!(h.due(DELAY_MS * 3.0), None, "once per visit");
        h.feed(Some((1, 4)), 1000.0);
        assert_eq!(h.due(1000.0 + DELAY_MS / 2.0), None, "a new element restarts the wait");
        h.feed(Some((1, 3)), 1400.0);
        assert_eq!(h.due(1400.0 + DELAY_MS), Some((1, 3)), "coming back speaks it again");
        h.feed(None, 3000.0);
        assert_eq!(h.due(9000.0), None, "over nothing, nothing is spoken");
    }
}
