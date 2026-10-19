//! The shell as a client of the audio service: Now Playing on the lock screen and in Control
//! Center, the transport buttons and the volume all talk to `bifrostd`'s audio service when it
//! has something loaded (Music is a client of the same service, `apps/music-desktop`).

use super::music::nowplaying::{self, Layout, LockCard, LOCK_ART_PT, LOCK_CARD_W_PT};
use super::music::{draw_cc_controls, LockCardView};
use super::ui::{Ui, Weight, WHITE};
use super::{Effect, Event, Shell};
use crate::gfx::{rgba, Image, Surface};
use crate::layout::Rect;
use crate::shell::switcher::draw_scaled;
use crate::svc::audio::{Cmd, Snapshot, Track};

/// Title and artist of the service's current track, rendered for the card and the tile.
pub(super) struct SvcLabels {
    key: (u64, String, Option<String>, u32),
    lock_title: Image,
    lock_artist: Image,
    cc_title: Image,
    cc_artist: Image,
}

/// A finger on the lock-screen card while it shows the service's track.
#[derive(Default)]
pub(super) struct SvcTouch {
    pressed: Option<nowplaying::Target>,
    scrub: Option<f32>,
}

/// The ID3 title and artist of a file, else its name; empty artist when unknown.
fn file_texts(path: &str) -> (String, String) {
    use std::io::Read;
    let stem = || std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string();
    let tags = (|| {
        let mut f = std::fs::File::open(path).ok()?;
        let mut head = [0u8; 10];
        f.read_exact(&mut head).ok()?;
        let len = crate::media::id3::tag_len(&head).min(2 << 20);
        if len == 0 {
            return None;
        }
        let mut bytes = head.to_vec();
        f.take(len as u64 - 10).read_to_end(&mut bytes).ok()?;
        crate::media::id3::parse(&bytes)
    })();
    match tags {
        Some(t) => (t.title.filter(|s| !s.is_empty()).unwrap_or_else(stem), t.artist.unwrap_or_default()),
        None => (stem(), String::new()),
    }
}

fn texts(track: &Track, snap: &Snapshot) -> (String, String) {
    if track.url.is_none() {
        return file_texts(&track.path);
    }
    let sub = match snap.live.as_ref() {
        Some(l) if l.phase == 0 => "Buffering…".to_string(),
        Some(l) if l.phase >= 2 => "Reconnecting…".to_string(),
        Some(l) => l.title.clone().unwrap_or_else(|| "Live".into()),
        None => "Live".into(),
    };
    ("Radio".into(), sub)
}

impl Shell {
    /// Whether the service holds a track (Now Playing and the transport buttons show it).
    pub(super) fn svc_active(&self) -> bool {
        self.audio.connected() && self.audio.snapshot().track.is_some()
    }

    /// Re-renders the card labels when the service's track changed.
    fn refresh_svc_labels(&mut self) {
        let snap = self.audio.snapshot();
        let Some(track) = snap.track.as_ref() else {
            self.svc_labels = None;
            return;
        };
        let live = snap.live.as_ref();
        let key = (track.id, track.path.clone(), live.and_then(|l| l.title.clone()), live.map_or(9, |l| l.phase.min(2)));
        if self.svc_labels.as_ref().is_some_and(|l| l.key == key) {
            return;
        }
        let (title, artist) = texts(track, snap);
        let ui = &self.ui;
        let lock_w = ui.pt(LOCK_CARD_W_PT - 32.0 - LOCK_ART_PT - 14.0);
        let cc_w = ui.pt(130.0);
        let dim = rgba(255, 255, 255, 150);
        self.svc_labels = Some(SvcLabels {
            key,
            lock_title: ui.text(&ui.fit(&title, 15.0, lock_w), 15.0, Weight::Semibold, WHITE),
            lock_artist: ui.text(&ui.fit(&artist, 13.0, lock_w), 13.0, Weight::Regular, dim),
            cc_title: ui.text(&ui.fit(&title, 15.0, cc_w), 15.0, Weight::Semibold, WHITE),
            cc_artist: ui.text(&ui.fit(&artist, 13.0, cc_w), 13.0, Weight::Regular, dim),
        });
    }

    /// After (re)connecting the service starts muted at full volume: tell it what the shell has.
    fn sync_audio_levels(&mut self) {
        if self.audio.connects() != self.audio_synced {
            self.audio_synced = self.audio.connects();
            self.audio.set_muted(self.music_muted);
            self.audio.set_volume(self.cc.volume);
            // the route as it is now, so a service that restarted tells its subscribers the truth (reason 0: unknown)
            self.audio.set_route(self.cc.route == crate::media::route::Route::Headphones, 0);
        }
    }

    fn svc_visible(&self) -> bool {
        self.screen == Screen::Lock || self.cc_active()
    }

    fn svc_changed(&mut self) {
        self.refresh_svc_labels();
        if self.svc_visible() {
            self.dirty = true;
            if self.screen == Screen::Lock {
                self.chrome_gen += 1;
            }
        }
    }

    /// The service sent something; true when the screen needs a new frame.
    pub(super) fn audio_pump(&mut self) -> bool {
        let c = self.audio.pump();
        if c {
            self.sync_audio_levels();
            self.svc_changed();
        }
        c && self.svc_visible()
    }

    pub(super) fn audio_tick(&mut self, t: f64) {
        if self.audio.tick(t) {
            self.sync_audio_levels();
            self.svc_changed();
        }
    }

    pub(super) fn audio_next_wake(&self, t: f64) -> Option<f64> {
        let reconnect = self.screen_on.then(|| self.audio.next_wake(t)).flatten();
        let moving = (self.screen_on && self.svc_visible() && self.audio.snapshot().playing && self.audio.snapshot().track.is_some()).then_some(t + 250.0);
        match (reconnect, moving) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The lock-screen card for the service's track.
    pub(super) fn svc_lock_card(&self) -> Option<LockCardView<'_>> {
        let labels = self.svc_labels.as_ref()?;
        Some(LockCardView::service(&self.ui, self.w, &labels.lock_title, &labels.lock_artist, self.audio.snapshot(), self.svc_touch.scrub, self.svc_touch.pressed))
    }

    pub(super) fn svc_transport(&mut self, target: nowplaying::Target) {
        match target {
            nowplaying::Target::Play => self.audio.toggle(),
            nowplaying::Target::Prev => self.audio.prev(),
            nowplaying::Target::Next => self.audio.next(),
            _ => false,
        };
    }

    /// A tap that ends on a lock-card control: transport, a feedback toggle (sent to the owner, which decides), or the
    /// rating at `x` on the rating track.
    fn svc_lock_action(&mut self, target: nowplaying::Target, card: &LockCard, x: f32) {
        use crate::shell::services::now_playing;
        let feedback = now_playing::feedback();
        match target {
            nowplaying::Target::Like | nowplaying::Target::Dislike | nowplaying::Target::Bookmark => {
                let slot: usize = match target {
                    nowplaying::Target::Like => 0,
                    nowplaying::Target::Dislike => 1,
                    _ => 2,
                };
                let active = feedback.is_some_and(|f| f.toggles[slot]);
                now_playing::set_toggle(slot, !active);
            }
            nowplaying::Target::Rating => {
                if let Some(scale) = feedback.and_then(|f| f.rating) {
                    let value = scale.min + f64::from(Layout::fraction(card.rating, x)) * (scale.max - scale.min);
                    now_playing::set_rating(value);
                }
            }
            other => self.svc_transport(other),
        }
    }

    /// A touch on the lock-screen card showing the service's track.
    pub(super) fn svc_lock_touch(&mut self, ev: Event) -> Option<Vec<Effect>> {
        let card = LockCard::new(&self.ui, self.w);
        let live = self.audio.snapshot().live.is_some();
        let fx = match ev {
            Event::Down(x, y) => {
                let hit = card.hit(x, y)?;
                if hit == nowplaying::Target::Scrubber {
                    if !live {
                        self.svc_touch.scrub = Some(Layout::fraction(card.scrub, x));
                    }
                } else {
                    self.svc_touch.pressed = Some(hit);
                }
                vec![]
            }
            Event::Move(x, _) if self.svc_touch.scrub.is_some() => {
                self.svc_touch.scrub = Some(Layout::fraction(card.scrub, x));
                vec![]
            }
            Event::Move(..) => {
                self.svc_touch.pressed?;
                vec![]
            }
            Event::Up(x, y) => {
                if let Some(f) = self.svc_touch.scrub.take() {
                    if let Some(d) = self.audio.snapshot().duration_ms {
                        self.audio.seek_ms(f as f64 * d);
                    }
                } else {
                    let pressed = self.svc_touch.pressed.take()?;
                    if card.hit(x, y) == Some(pressed) {
                        self.svc_lock_action(pressed, &card, x);
                    }
                }
                vec![]
            }
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return None,
        };
        self.dirty = true;
        self.chrome_gen += 1;
        Some(fx)
    }

    /// The Control Center tile for the service's track.
    pub(super) fn draw_svc_tile(&self, s: &mut Surface, (x, y, w, h): (f32, f32, f32, f32), alpha: u32, pressed: Option<nowplaying::Target>) -> bool {
        let Some(labels) = self.svc_labels.as_ref() else { return false };
        let ui: &Ui = &self.ui;
        let size = ui.pt(44.0);
        let r = Rect { x: (x + ui.pt(14.0)) as i32, y: (y + ui.pt(14.0)) as i32, w: size as i32, h: size as i32 };
        draw_scaled(s, &super::music::icons::placeholder(64, 0.0), r, ui.pt(5.0), alpha);
        let tx = (x + ui.pt(14.0) + size + ui.pt(10.0)) as i32;
        s.blit(&labels.cc_title, tx, (y + ui.pt(16.0)) as i32, 255, alpha);
        s.blit(&labels.cc_artist, tx, (y + ui.pt(38.0)) as i32, 255, alpha);
        draw_cc_controls(s, ui, (x, y, w, h), self.audio.snapshot().playing, crate::gfx::scale(WHITE, alpha), pressed);
        true
    }

    /// Hardware keys and the slider reach the service too.
    pub(super) fn audio_volume(&mut self, level: f32) {
        self.audio.set_volume(level);
    }

    pub(super) fn audio_pause(&mut self) {
        self.audio.send(Cmd::Pause);
    }

    /// Lets the lock-screen card take a touch before the unlock drag does.
    pub(crate) fn music_lock_event(&mut self, ev: Event) -> Option<Vec<Effect>> {
        if !self.svc_active() {
            return None;
        }
        self.svc_lock_touch(ev)
    }

    /// A transport control from the Control Center media tile.
    pub(crate) fn music_control(&mut self, target: nowplaying::Target) {
        if self.svc_active() {
            self.svc_transport(target);
        }
    }

    /// Paints the media tile at `rect` inside the Control Center sheet.
    pub(crate) fn draw_music_tile(&self, s: &mut Surface, rect: (f32, f32, f32, f32), alpha: u32, pressed: Option<nowplaying::Target>) {
        if self.svc_active() && self.draw_svc_tile(s, rect, alpha, pressed) {
            return;
        }
        let img = self.ui.text("Not Playing", 17.0, Weight::Semibold, rgba(255, 255, 255, 140));
        s.blit(&img, (rect.0 + rect.2 / 2.0 - img.w as f32 / 2.0) as i32, (rect.1 + self.ui.pt(28.0)) as i32, 255, alpha);
        draw_cc_controls(s, &self.ui, rect, false, crate::gfx::scale(rgba(255, 255, 255, 90), alpha), pressed);
    }

    /// Whether the speaker stays silent (true) or plays (false, the default).
    pub fn set_music_muted(&mut self, muted: bool) {
        self.music_muted = muted;
        self.audio.set_muted(muted);
    }
}

use super::Screen;
