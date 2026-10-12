//! Shell side of sharing: opening the sheet, the nearby service, the
//! incoming-file prompt and the modal touch handling. Kept out of
//! shell/mod.rs so that file only gains the fields and a few hook lines.

use super::prompt::{Choice, Prompt};
use super::{Item, Outcome, Sheet};
use crate::net::share::{self, Event};
use crate::shell::notify::store::Notification;
use crate::shell::{AppId, Effect, Event as Touch, Shell};
use std::path::PathBuf;

/// Where received files and "Save to Files" copies land.
pub fn received_dir() -> PathBuf {
    crate::files::fs::home().join("Received")
}

fn device_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "BifrOSt".into())
}

/// Sends an item on a worker thread; the result is reported through the log.
fn send_in_background(item: Item, secret: String, peer: share::Peer) {
    std::thread::spawn(move || {
        let r = match (&item, item.bytes()) {
            (_, Some(bytes)) => share::send_bytes(peer.addr, &secret, &item.file_name(), &bytes),
            (Item::File(p), None) => std::fs::File::open(p).and_then(|mut f| share::send(peer.addr, &secret, &item.file_name(), &mut f, |_, _| {})),
            _ => Ok(()),
        };
        if let Err(e) = r {
            eprintln!("springboard: share to {}: {e}", peer.name);
        }
    });
}

fn save_to_files(item: &Item) -> std::io::Result<PathBuf> {
    let dir = received_dir();
    std::fs::create_dir_all(&dir)?;
    let (dest, mut out) = share::create_unique(&dir, &item.file_name())?;
    match (item.bytes(), item) {
        (Some(b), _) => std::io::Write::write_all(&mut out, &b)?,
        (None, Item::File(p)) => {
            std::io::copy(&mut std::fs::File::open(p)?, &mut out)?;
        }
        _ => {}
    }
    Ok(dest)
}

impl Shell {
    /// Opens the share sheet for `item`, below `anchor` (px), and starts the
    /// nearby service on first use.
    pub fn open_share(&mut self, item: Item, anchor: (f32, f32)) {
        self.ensure_share_service();
        let mut sheet = Sheet::new(item, anchor);
        if let Some(svc) = &self.share_svc {
            sheet.set_peers(svc.peers());
        }
        self.share = Some(sheet);
        self.drawn = crate::shell::Drawn::Nothing;
    }

    /// Starts announcing and listening; a failure (no network yet) is retried on the next call.
    pub fn ensure_share_service(&mut self) {
        if self.share_svc.is_none() {
            match share::Service::start(&device_name(), share::DEFAULT_SECRET, received_dir()) {
                Ok(s) => self.share_svc = Some(s),
                Err(e) => eprintln!("springboard: share service: {e}"),
            }
        }
    }

    /// Called about once a second: refreshes the nearby row and turns
    /// service events into the prompt and notifications.
    pub fn poll_share(&mut self) {
        let Some(svc) = &self.share_svc else { return };
        let peers = svc.peers();
        let events: Vec<_> = std::iter::from_fn(|| svc.events.try_recv().ok()).collect();
        let mut changed = !events.is_empty();
        if let Some(sheet) = &mut self.share {
            changed |= sheet.set_peers(peers.clone());
        }
        for ev in events {
            match ev {
                // General › Nearby: off declines everything, Contacts Only only devices already seen nearby
                Event::Offer(o) if self.options().nearby == 0 || (self.options().nearby == 1 && !peers.iter().any(|p| p.addr.ip() == o.from.ip())) => o.decline(),
                Event::Offer(o) if self.share_prompt.is_none() => self.share_prompt = Some(Prompt::new(o)),
                Event::Offer(o) => o.decline(),
                Event::Received(path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let now = self.notify.now();
                    self.notify(Notification::new(AppId::Files, "Received", name, now));
                }
                Event::Failed(e) => eprintln!("springboard: share: {e}"),
            }
        }
        if changed {
            self.drawn = crate::shell::Drawn::Nothing;
        }
    }

    /// Touches for the prompt and the sheet; None when neither is up.
    pub(crate) fn share_modal(&mut self, ev: Touch) -> Option<Vec<Effect>> {
        let Touch::Up(x, y) = ev else {
            return (self.share.is_some() || self.share_prompt.is_some()).then(Vec::new);
        };
        let (w, h) = (self.w, self.h);
        if self.share_prompt.is_some() {
            match Prompt::hit(&self.ui, w, h, x, y) {
                Some(c) => {
                    let p = self.share_prompt.take().unwrap();
                    if c == Choice::Accept { p.offer.accept() } else { p.offer.decline() }
                }
                None => {}
            }
            self.drawn = crate::shell::Drawn::Nothing;
            return Some(vec![]);
        }
        let sheet = self.share.as_ref()?;
        let outcome = sheet.tap(&self.ui, w, h, x, y);
        let secret = share::DEFAULT_SECRET.to_string();
        match outcome {
            Outcome::None => {}
            Outcome::Close => self.share = None,
            Outcome::Copy(item) => {
                crate::shell::clipboard::set_text(&item.clip_text());
                self.share = None;
            }
            Outcome::SaveToFiles(item) => {
                if let Err(e) = save_to_files(&item) {
                    eprintln!("springboard: save to files: {e}");
                }
                self.share = None;
            }
            Outcome::SendTo(item, peer) => {
                send_in_background(item, secret, peer);
                self.share = None;
            }
        }
        self.drawn = crate::shell::Drawn::Nothing;
        Some(vec![])
    }

    pub(crate) fn draw_share(&self, s: &mut crate::gfx::Surface<'_>) {
        if let Some(sheet) = &self.share {
            sheet.draw(s, &self.ui, self.w, self.h);
        }
        if let Some(p) = &self.share_prompt {
            p.draw(s, &self.ui, self.w, self.h);
        }
    }
}
