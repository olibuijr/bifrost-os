//! Renderer-independent controls shared by BifrOSt applications.
//! Geometry uses logical points; rendering adapters supply the system materials.
use std::ops::{Deref, Range};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Command {
    Cut = 0,
    Copy = 1,
    Paste = 2,
    SelectAll = 3,
    Undo = 4,
    Redo = 5,
}
impl Command {
    /// In wire order: the value of a command is its index here. New commands go at the end.
    pub const ALL: [Self; 6] = [Self::Cut, Self::Copy, Self::Paste, Self::SelectAll, Self::Undo, Self::Redo];
    pub fn bit(self) -> u32 {
        1 << self as u32
    }
    pub fn from_wire(value: u32) -> Option<Self> {
        Self::ALL.get(value as usize).copied()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Commands(pub u32);
impl Commands {
    pub fn supports(self, command: Command) -> bool {
        self.0 & command.bit() != 0
    }
    pub fn with(mut self, command: Command, enabled: bool) -> Self {
        if enabled {
            self.0 |= command.bit();
        }
        self
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Role {
    Button = 0,
    TextField = 1,
    List = 2,
    Row = 3,
    /// A binary on/off control (`NativeSwitch` / `NativeSwitch`).
    Toggle = 4,
    /// A binary checked control (`NativeButton` checkbox style).
    Checkbox = 5,
    /// A numeric range control (`NativeSlider`).
    Slider = 6,
    /// A mutually exclusive choice group (`NativeSegmentedControl`).
    SegmentedControl = 7,
    /// Non-interactive, readable text (`NativeAccessibilityStaticText`).
    StaticText = 8,
    /// A command in a menu (`NativeMenuItem`).
    MenuItem = 9,
    /// A top-level application window (`NativeWindow`).
    Window = 10,
    /// A determinate or indeterminate progress display (`NativeProgressIndicator`).
    ProgressIndicator = 11,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u64,
    pub role: Role,
    pub label: String,
    pub value: String,
    pub bounds: Rect,
    pub enabled: bool,
}
#[derive(Default, Debug)]
pub struct Controls {
    nodes: Vec<Node>,
    focus: Option<u64>,
    pressed: Option<u64>,
}
impl Controls {
    pub fn update(&mut self, nodes: Vec<Node>) {
        self.nodes = nodes;
        if self
            .focus
            .is_some_and(|id| !self.nodes.iter().any(|n| n.id == id && n.enabled))
        {
            self.focus = None;
        }
        if self
            .pressed
            .is_some_and(|id| !self.nodes.iter().any(|n| n.id == id && n.enabled))
        {
            self.pressed = None;
        }
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn focused(&self) -> Option<&Node> {
        self.focus
            .and_then(|id| self.nodes.iter().find(|n| n.id == id))
    }
    pub fn focus(&mut self, id: Option<u64>) {
        self.focus = id.filter(|id| self.nodes.iter().any(|n| n.id == *id && n.enabled));
    }
    pub fn advance(&mut self, backwards: bool) -> Option<u64> {
        let ids: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| n.enabled && matches!(n.role, Role::Button | Role::TextField | Role::Toggle | Role::Checkbox | Role::Slider | Role::SegmentedControl | Role::MenuItem))
            .map(|n| n.id)
            .collect();
        if ids.is_empty() {
            self.focus = None;
            return None;
        }
        let next = match self.focus.and_then(|id| ids.iter().position(|n| *n == id)) {
            Some(i) => {
                if backwards {
                    (i + ids.len() - 1) % ids.len()
                } else {
                    (i + 1) % ids.len()
                }
            }
            None => {
                if backwards {
                    ids.len() - 1
                } else {
                    0
                }
            }
        };
        self.focus = Some(ids[next]);
        self.focus
    }
    pub fn down(&mut self, x: f32, y: f32) {
        self.pressed = self
            .nodes
            .iter()
            .rev()
            .find(|n| n.enabled && n.bounds.contains(x, y))
            .map(|n| n.id);
        self.focus(self.pressed);
    }
    pub fn up(&mut self, x: f32, y: f32) -> Option<u64> {
        let id = self.pressed.take()?;
        self.nodes
            .iter()
            .find(|n| n.id == id && n.enabled && n.bounds.contains(x, y))
            .map(|n| n.id)
    }
}
/// UTF-8 safe single-line editor. Selection and caret positions are byte boundaries.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    text: String,
    caret: usize,
    anchor: usize,
    /// States before each edit, oldest first (bounded by `UNDO_LEVELS`), and the ones undone since.
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    /// What the last edit was, so a run of typing becomes one undo step.
    last: Edit,
}

/// How many edits a field remembers.
pub const UNDO_LEVELS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    caret: usize,
    anchor: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Edit {
    #[default]
    None,
    Typing,
    Other,
}
impl Deref for TextField {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
impl std::fmt::Display for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}
impl TextField {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn caret(&self) -> usize {
        self.caret
    }
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }
    /// Empties the field and forgets its history (a programmatic reset, not something to undo).
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.past.clear();
        self.future.clear();
        self.last = Edit::None;
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), caret: self.caret, anchor: self.anchor }
    }
    /// Remembers the state before an edit. Consecutive single-character typing with nothing selected joins the
    /// previous step (as `NativeTextView` groups typing); anything else starts a new one. A new edit drops the redo history.
    fn record(&mut self, kind: Edit) {
        let joins = kind == Edit::Typing && self.last == Edit::Typing && self.selection().is_empty();
        if !joins {
            let snap = self.snapshot();
            self.past.push(snap);
            if self.past.len() > UNDO_LEVELS {
                self.past.remove(0);
            }
        }
        self.future.clear();
        self.last = kind;
    }
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.caret = s.caret;
        self.anchor = s.anchor;
        self.last = Edit::None;
    }
    /// Reverses the latest edit (a whole run of typing at once); true when something changed.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.past.pop() else { return false };
        let now = self.snapshot();
        self.future.push(now);
        self.restore(prev);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else { return false };
        let now = self.snapshot();
        self.past.push(now);
        self.restore(next);
        true
    }
    pub fn push_str(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() && self.selection().is_empty() {
            return;
        }
        self.record(if clean.chars().count() == 1 { Edit::Typing } else { Edit::Other });
        let range = self.selection();
        let start = range.start;
        self.text.replace_range(range, &clean);
        self.caret = start + clean.len();
        self.anchor = self.caret;
    }
    pub fn pop(&mut self) {
        if self.text.is_empty() || (self.selection().is_empty() && self.caret == 0) {
            return;
        }
        self.record(Edit::Other);
        let mut range = self.selection();
        if range.is_empty() {
            range.start = self.text[..self.caret]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = self.caret;
    }
    pub fn delete(&mut self) {
        if self.selection().is_empty() && self.caret >= self.text.len() {
            return;
        }
        self.record(Edit::Other);
        let mut range = self.selection();
        if range.is_empty() {
            range.end = self.caret
                + self.text[self.caret..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
        }
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = self.caret;
    }
    pub fn move_caret(&mut self, right: bool) {
        self.last = Edit::None;
        let r = self.selection();
        self.caret = if !r.is_empty() {
            if right {
                r.end
            } else {
                r.start
            }
        } else if right {
            self.caret
                + self.text[self.caret..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0)
        } else {
            self.text[..self.caret]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0)
        };
        self.anchor = self.caret;
    }
    pub fn home(&mut self) {
        self.last = Edit::None;
        self.caret = 0;
        self.anchor = 0;
    }
    pub fn end(&mut self) {
        self.last = Edit::None;
        self.caret = self.text.len();
        self.anchor = self.caret;
    }
    pub fn commands(&self, clipboard: bool) -> Commands {
        Commands::default()
            .with(Command::Cut, !self.selection().is_empty())
            .with(Command::Copy, !self.selection().is_empty())
            .with(Command::Paste, clipboard)
            .with(Command::SelectAll, !self.text.is_empty())
            .with(Command::Undo, self.can_undo())
            .with(Command::Redo, self.can_redo())
    }
    /// Returns clipboard replacement for Cut/Copy; Paste receives existing clipboard text.
    pub fn command(&mut self, command: Command, clipboard: &str) -> Option<String> {
        match command {
            Command::SelectAll => {
                self.last = Edit::None;
                self.anchor = 0;
                self.caret = self.text.len();
                None
            }
            Command::Undo => {
                self.undo();
                None
            }
            Command::Redo => {
                self.redo();
                None
            }
            Command::Copy => Some(self.selected_text().into()),
            Command::Cut => {
                let copied = self.selected_text().to_owned();
                if !copied.is_empty() {
                    self.record(Edit::Other);
                }
                let range = self.selection();
                self.text.replace_range(range.clone(), "");
                self.caret = range.start;
                self.anchor = self.caret;
                Some(copied)
            }
            Command::Paste => {
                self.push_str(clipboard);
                None
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_unicode_selection_and_clipboard() {
        let mut f = TextField::new();
        f.push_str("café🦀");
        f.move_caret(false);
        f.pop();
        assert_eq!(&*f, "caf🦀");
        f.command(Command::SelectAll, "");
        assert_eq!(f.command(Command::Cut, "").unwrap(), "caf🦀");
        f.command(Command::Paste, "new\ntext");
        assert_eq!(&*f, "newtext");
        f.home();
        f.delete();
        assert_eq!(&*f, "ewtext");
    }
    #[test]
    fn focus_skips_disabled_and_pointer_release_outside_does_not_activate() {
        let mut c = Controls::default();
        c.update(vec![
            Node {
                id: 1,
                role: Role::Button,
                label: "Back".into(),
                value: String::new(),
                bounds: Rect {
                    x: 0.,
                    y: 0.,
                    w: 44.,
                    h: 44.,
                },
                enabled: false,
            },
            Node {
                id: 2,
                role: Role::TextField,
                label: "Search".into(),
                value: String::new(),
                bounds: Rect {
                    x: 44.,
                    y: 0.,
                    w: 100.,
                    h: 44.,
                },
                enabled: true,
            },
        ]);
        assert_eq!(c.advance(false), Some(2));
        c.down(50., 20.);
        assert_eq!(c.up(200., 20.), None);
        c.update(vec![]);
        assert!(c.focused().is_none());
    }

    fn typed(f: &mut TextField, s: &str) {
        for c in s.chars() {
            f.push_str(&c.to_string());
        }
    }

    #[test]
    fn a_run_of_typing_is_one_undo_step_and_redo_brings_it_back() {
        let mut f = TextField::new();
        typed(&mut f, "hello");
        assert_eq!(&*f, "hello");
        assert!(f.undo());
        assert_eq!((&*f, f.caret()), ("", 0), "all five letters went in one undo");
        assert!(!f.undo(), "nothing left");
        assert!(f.redo());
        assert_eq!((&*f, f.caret()), ("hello", 5));
        assert!(!f.redo());
    }

    #[test]
    fn moving_the_caret_ends_a_typing_group() {
        let mut f = TextField::new();
        typed(&mut f, "ab");
        f.move_caret(false);
        typed(&mut f, "X");
        assert_eq!(&*f, "aXb");
        f.undo();
        assert_eq!(&*f, "ab", "only the X after the move is undone");
        f.undo();
        assert_eq!(&*f, "");
    }

    #[test]
    fn deletes_and_cuts_are_their_own_steps_and_restore_caret_and_selection() {
        let mut f = TextField::new();
        f.push_str("hello world");
        f.command(Command::SelectAll, "");
        f.command(Command::Cut, "");
        assert_eq!(&*f, "");
        f.undo();
        assert_eq!((&*f, f.selection()), ("hello world", 0..11), "the selection comes back too");
        f.end();
        f.pop();
        f.pop();
        assert_eq!(&*f, "hello wor");
        f.undo();
        assert_eq!(&*f, "hello worl", "each backspace is a step");
        f.undo();
        assert_eq!(&*f, "hello world");
    }

    #[test]
    fn paste_over_a_selection_undoes_to_the_selected_text() {
        let mut f = TextField::new();
        f.push_str("abcdef");
        f.home();
        f.move_caret(true);
        f.anchor = 1;
        f.caret = 4;
        f.command(Command::Paste, "XY");
        assert_eq!(&*f, "aXYef");
        f.undo();
        assert_eq!((&*f, f.selection()), ("abcdef", 1..4));
    }

    #[test]
    fn a_new_edit_clears_redo_clear_resets_history_and_the_history_is_bounded() {
        let mut f = TextField::new();
        f.push_str("one");
        f.undo();
        assert!(f.can_redo());
        f.push_str("two");
        assert!(!f.can_redo());
        f.clear();
        assert!(!f.can_undo() && !f.can_redo());
        for i in 0..(UNDO_LEVELS + 20) {
            f.push_str(&format!("{i:03}"));
        }
        let mut n = 0;
        while f.undo() {
            n += 1;
        }
        assert_eq!(n, UNDO_LEVELS);
    }

    #[test]
    fn commands_advertise_undo_and_redo_only_when_there_is_history() {
        let mut f = TextField::new();
        assert!(!f.commands(false).supports(Command::Undo) && !f.commands(false).supports(Command::Redo));
        typed(&mut f, "x");
        assert!(f.commands(false).supports(Command::Undo));
        f.command(Command::Undo, "");
        assert!(!f.commands(false).supports(Command::Undo) && f.commands(false).supports(Command::Redo));
        f.command(Command::Redo, "");
        assert_eq!(&*f, "x");
        assert_eq!(Command::from_wire(4), Some(Command::Undo));
        assert_eq!(Command::from_wire(5), Some(Command::Redo));
        assert_eq!(Command::from_wire(6), None, "an unknown command is refused, not guessed");
    }
}

/// Shared list scroll physics.
pub mod scroll;
pub mod geometry;
pub mod stack;
pub mod split;
pub mod values;
pub mod vector_ops;
pub mod quadrature;
pub mod lu;
pub mod nav;
pub mod responder;
pub mod undo;
pub mod text;
pub mod keys;
pub mod anim;
pub mod symbol_effect;
pub mod frame_rate;
pub mod sidebar;
pub mod diff;
pub mod cache;
pub mod fuzzy;
pub mod visual_search;
pub mod regex;
pub mod format;
pub mod gesture;
pub mod identity;
pub mod directory_node;
pub mod directory_query;
pub mod access_control;
pub mod trust;
pub mod focus;
pub mod pane;
pub mod panner;
pub mod param_view;
pub mod uttype;
pub mod preview;
pub mod thumbnail;
pub mod fileaction;
pub mod actionrequest;
pub mod textfit;
pub mod paragraph;
pub mod bidi;
pub mod observe;
pub mod controller;
pub mod controller_power;
pub mod calendar;
pub mod calendar_error;
pub mod calendar_store_changes;
pub mod collection;
pub mod dockdrag;
pub mod dock_tile;
pub mod cursor;
pub mod screen;
pub mod menu;
pub mod launch;
pub mod detect;
pub mod prefs_layout;
pub mod search;
pub mod textfind;
pub mod textlist;
pub mod metaquery;
pub mod searchquery;
pub mod pathbar;
pub mod constraint;
pub mod table;
pub mod panels;
pub mod sheet;
pub mod linalg;
pub mod scene_object;
pub mod view_camera;
pub mod animated_values;
pub mod mesh;
pub mod shading;
pub mod composition;
pub mod candidates;
pub mod audio_mix;
pub mod audio_render;
pub mod plugin_host;
pub mod plugin_view;
pub mod state_machine;
pub mod agents;
pub mod grid_graph;
pub mod noise;
pub mod quadtree;
pub mod rule_system;
pub mod tensor;
pub mod graph;
pub mod nn;
pub mod save_sync;
pub mod ubiquitous_store;
pub mod gamepad;
pub mod affine;
pub mod pdfpage;
pub mod pdfselect;
pub mod pdfannot;
pub mod transform3d;
pub mod scene3d;
pub mod path;
pub mod gradient;
pub mod clip;
pub mod render_loop;
pub mod drawable;
pub mod workflow;
pub mod scripting;
pub mod subscription;
pub mod auth_view;
pub mod one_time_code;
pub mod entitlement;
pub mod purchase;
pub mod offer;
pub mod download;
pub mod nav_policy;
pub mod translation;
pub mod timeline;
pub mod relevance;
pub mod intent_resolve;
pub mod intent_vocab;
pub mod intent_access;
pub mod widget_action;
pub mod skip_interval;
pub mod reading_list;
pub mod content_rules;
pub mod tips;
pub mod chart;
pub mod chart_ax;
pub mod chart_marks;
pub mod compose;
pub mod file_provider;
pub mod widget;
pub mod widget_list;
pub mod widget_link;
pub mod widget_ax;
pub mod media_query;
pub mod media_search;
pub mod media_request;
pub mod media_feedback;
pub mod media_selection;
pub mod media_sources;
pub mod playback_rate;
pub mod extension;
pub mod extensions;
pub mod clip_buffer;
pub mod recording;
pub mod script;
pub mod writing_assist;
pub mod vcard;
pub mod contact_groups;
pub mod contact_changes;
pub mod contact_card;
pub mod contact_picker;
pub mod contact_access;
pub mod contact_fetch;
pub mod postal_address;
pub mod fetch;
pub mod history;
pub mod model_history;
pub mod shape;
pub mod assetpack;
pub mod downloadqueue;
pub mod environment;
pub mod preferences;
pub mod call;
pub mod hfp;
pub mod markup;
pub mod drawing;
pub mod markup_drawing;
pub mod lasso;
pub mod transcript;
pub mod context_window;
pub mod generation_schema;
pub mod model_session;
pub mod music;
pub mod indexset;
pub mod tracking;
pub mod evaluation;
pub mod notifattach;
pub mod notifmedia;
pub mod progress;
pub mod privacy;
pub mod relations;
pub mod model_store;
pub mod ml_tensor;
pub mod features;
pub mod compute;
pub mod ml_model;
pub mod ml_state;
pub mod work_stream;
pub mod postal;
pub mod recordsearch;
pub mod file_sync;
pub mod item_attrs;
#[cfg(test)]
mod bench;

pub fn rubber_band(over: f32, limit: f32) -> f32 {
    if limit <= 0.0 {
        return 0.0;
    }
    let a = over.abs();
    over.signum() * (1.0 - 1.0 / (a * 0.55 / limit + 1.0)) * limit
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Down(f32, f32),
    Move(f32, f32),
    Up(f32, f32),
    Power,
    /// A wake request (touch-controller gesture, wake key): turns the
    /// screen on, never off.
    Wake,
    /// Hardware volume keys: one bifrost step, on every screen, never waking it.
    VolumeUp,
    VolumeDown,
    /// Power + Volume Up: save the screen as a PNG in Photos.
    Screenshot,
}
pub mod ax;
pub mod text_marker;
pub mod speech;
pub mod recognition;
pub mod voice_activity;
pub mod sound_classify;
pub mod sound_envelope;
pub mod event_envelope;
pub mod event_distance;
pub mod sound_distance;
pub mod sound_event;
pub mod analyzer_input;
pub mod sound_result;
pub mod sound_analyzer;
pub mod audio_session;
pub mod playback_arbiter;
pub mod remote_command;
pub mod media_content;
pub mod audio_hardware;
pub mod process_tap;
pub mod scan;
pub mod analysis;
pub mod docscan;
pub mod device_browser;
pub mod scan_job;
pub mod camera_device;
pub mod capture_sync;
pub mod geo;
pub mod record_store;
pub mod record_sync;
pub mod record_asset;
pub mod weather_model;
pub mod language;
pub mod segment;
pub mod location;
pub mod geofence;
pub mod map_overlay;
pub mod poi_filter;
pub mod route;
pub mod mapview;
pub mod mapshape;
pub mod reverse_lookup;
pub mod switch_scan;
pub mod ax_audit;
pub mod state_report;
pub mod precipitation;
pub mod weather_alert;
pub mod weather_attribution;
pub mod pressure_trend;
pub mod weather_stats;
pub mod hid_report;
pub mod linkmeta;
pub mod captions;
pub mod blit;
pub mod vertex_descriptor;
pub mod gatt;
pub mod pip_window;
pub mod content_proposal;
pub mod route_auth;
pub mod route_action;
pub mod route_picker;
pub mod webauth_session;
pub mod verification_codes;
pub mod photo_changes;
pub mod photo_media;
pub mod photo_request;
pub mod fs_volume;
pub mod fs_probe;
pub mod fs_maintenance;
pub mod keychain_save;
pub mod keychain_settings;
pub mod certificate_trust;
pub mod vm_state;
pub mod vm_share;
pub mod vm_error;
pub mod smartcard_slot;
pub mod token_watch;
pub mod screensaver;
pub mod disk_image;

// Media analysis: keyframe, highlight and face grouping models (parity wave 3)
pub mod keyframe_pick;
pub mod highlight_rank;
pub mod face_group;
pub mod emitter;
pub mod physics;
pub mod tilemap;
pub mod midi_thru;
pub mod midi_events;
pub mod fourier;
pub mod embedding;
pub mod lexical_tags;
pub mod filebadge;
pub mod itemattrs;
pub mod match_pattern;
pub mod url_filter;
pub mod call_directory;
pub mod annotation;
pub mod ecs;
pub mod rules;
pub mod decision;
