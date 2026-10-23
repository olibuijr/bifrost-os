//! Files's drag and drop with other windows and apps (`dnd`). A drag that starts on items asks the shell for a read grant per
//! item (`Ctx::grant_path`); once they are all granted the window begins a cross-window session with them, and the shell owns
//! the pointer until it ends. While it does, the window hears its own drops (`Dest`), and it is a drop target for the files
//! other apps drop on it: the files arrive as grants, are held until the drop is performed, then are copied into the folder.
//! Until the grants arrive the drag is this window's own and ends in the in-window drop, as before.
use crate::*;
use crate::dragpreview::drag_preview;
use bifrost_app::dnd::{self, DragItem, DragPhase, DragPreview, Payload};
use bifrost_app::proto::GrantId;
use bifrost_app::{Document, DocumentEvent};

/// Whether a drop of this window's own drag copies: Option was held (`copy_drag`), or the shell narrowed the mask so that
/// a move is not allowed (Option held while the shell owns the pointer).
pub(crate) fn own_drop_copies(copy_drag: bool, allowed: u8) -> bool {
    copy_drag || allowed & dnd::dragop::MOVE == 0
}

/// The operation this window answers for a hover: none away from a folder; inside the window a move, or a copy when
/// `copy`; files from another app are always copied.
pub(crate) fn drop_reply(over_folder: bool, own: bool, copy: bool) -> u8 {
    match (over_folder, own) {
        (false, _) => 0,
        (true, true) if copy => dnd::dragop::COPY,
        (true, true) => dnd::dragop::MOVE,
        (true, false) => dnd::dragop::COPY,
    }
}

/// Whether an `Ended` for this window's own session means the shell refused it: no `Dest` event came for it while the
/// window still drags. Then the touches stay with the window and its own drop handles the release.
pub(crate) fn session_refused(heard: bool, dragging: bool) -> bool {
    !heard && dragging
}

/// The cross-window drag state of one window.
#[derive(Default)]
pub(crate) struct DragOut {
    /// grant requests of the dragged items that have not been answered
    asking: Vec<u32>,
    /// the grants answered for this drag: grant and file name
    granted: Vec<(GrantId, String)>,
    /// a grant request was refused: no session for this drag
    failed: bool,
    /// the session this window began; its grants go back when it ends
    pub(crate) session: Option<u32>,
    session_grants: Vec<GrantId>,
    /// the shell sent this window a `Dest` event of its own session: the shell owns the pointer for it
    heard: bool,
    /// a drag from another app hovers over this window: its session, and how many items it carries
    pub(crate) foreign: Option<u32>,
    pub(crate) foreign_items: usize,
    /// files another app dropped here, held until the drop is performed: (session, name, file)
    held: Vec<(u32, String, std::fs::File)>,
    /// the drag image of the drag being asked for, given to the session when it begins
    preview: DragPreview,
}

impl Files {
    /// The drag just started on items: ask the shell for a read grant for each one. The session begins when they are all in.
    pub(crate) fn drag_out_ask(&mut self, ctx: &Ctx) {
        self.dnd.asking = self.fx.drag_paths.iter().map(|p| ctx.grant_path(&p.to_string_lossy())).collect();
        self.dnd.granted.clear();
        self.dnd.failed = false;
        self.dnd.preview = match self.fx.drag_paths.first() {
            Some(first) => {
                let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                drag_preview(springboard::files::fs::kind_of(&name, first.is_dir()), self.fx.drag_paths.len())
            }
            None => DragPreview::default(),
        };
    }

    /// Begins the session once every item of the drag is granted, if the drag is still going.
    fn drag_out_settle(&mut self, ctx: &Ctx) {
        if !self.dnd.asking.is_empty() {
            return;
        }
        let granted = std::mem::take(&mut self.dnd.granted);
        let failed = std::mem::take(&mut self.dnd.failed);
        if !self.dragging || failed || self.dnd.session.is_some() || granted.is_empty() {
            // the drag is over or cannot go out: the grants are not needed
            for (grant, _) in granted {
                ctx.release_document(grant);
            }
            return;
        }
        let grants: Vec<GrantId> = granted.iter().map(|(g, _)| *g).collect();
        let items: Vec<DragItem> = granted.into_iter().map(|(grant, name)| DragItem::new(Payload::File { grant, name })).collect();
        let session = dnd::begin(ctx, dnd::dragop::COPY | dnd::dragop::MOVE, items, std::mem::take(&mut self.dnd.preview));
        self.dnd.session = Some(session);
        self.dnd.session_grants = grants;
        self.dnd.heard = false;
    }

    /// `Ctx` document answers: grants for the drag, the files a drop brings, and the refusals.
    pub(crate) fn drag_out_document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) => {
                let Document { request, grant, name, file, .. } = doc;
                if let Some(i) = self.dnd.asking.iter().position(|r| *r == request) {
                    self.dnd.asking.remove(i);
                    self.dnd.granted.push((grant, name));
                    drop(file);
                    self.drag_out_settle(ctx);
                } else if request & bifrost_app::share::INBOUND != 0 {
                    self.drag_out_arrived(request, vec![(name, file)]);
                } else {
                    // a grant from an earlier drag that was not waited for
                    ctx.release_document(grant);
                }
                true
            }
            DocumentEvent::OpenedMany { request, docs } => {
                let files = docs.into_iter().map(|d| (d.name, d.file)).collect();
                self.drag_out_arrived(request, files);
                true
            }
            DocumentEvent::Failed { request, .. } => {
                if let Some(i) = self.dnd.asking.iter().position(|r| *r == request) {
                    self.dnd.asking.remove(i);
                    self.dnd.failed = true;
                    self.drag_out_settle(ctx);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Files another app sent: held for the drop that brings them, or, with no drop waiting (a Dock icon's Open With), copied
    /// into the folder shown.
    fn drag_out_arrived(&mut self, request: u32, files: Vec<(String, std::fs::File)>) {
        if self.dnd.foreign == Some(request) {
            self.dnd.held.extend(files.into_iter().map(|(name, file)| (request, name, file)));
            return;
        }
        if self.in_recents() {
            return;
        }
        let dir = self.path.clone();
        self.start_receive("Copying", files, dir);
        self.revision += 1;
    }

    /// Pointer hovers, enters, leaves or drops on this window during a session (own or another app's).
    /// `allowed` is the source's mask as the held modifier keys narrow it (Option leaves copy only).
    pub(crate) fn drag_out_dest(&mut self, ctx: &Ctx, session: u32, phase: DragPhase, x: f32, y: f32, allowed: u8, items: Vec<DragItem>) -> bool {
        let own = self.dnd.session == Some(session);
        if own {
            self.dnd.heard = true;
        }
        // a drag inside this window moves; Option (the key, or a mask the shell narrowed to copy) copies
        let copy = own_drop_copies(self.fx.copy_drag, allowed);
        match phase {
            DragPhase::Entered | DragPhase::Updated => {
                if !own {
                    self.dragging = true;
                    self.dnd.foreign = Some(session);
                    if !items.is_empty() {
                        self.dnd.foreign_items = items.len();
                    }
                }
                let target = self.drop_target_at(x, y);
                if target != self.drop_target {
                    self.spring_hover(target.as_ref());
                    self.drop_target = target;
                }
                self.revision += 1;
                // inside this window a drag moves the items, Option copies; files from another app are copied
                dnd::reply(ctx, session, drop_reply(self.drop_target.is_some(), own, copy));
                true
            }
            DragPhase::Exited => {
                self.drop_target = None;
                self.spring_hover(None);
                self.drag_out_foreign_over(session);
                self.revision += 1;
                true
            }
            DragPhase::Perform => {
                let dest = self.drop_target.take();
                self.spring_hover(None);
                if own {
                    // a drop on one of this window's folders (the shell always hands the source window its own drop): the
                    // target from the last Updated, else the folder under the drop point
                    let dest = dest.or_else(|| self.drop_target_at(x, y));
                    self.dragging = false;
                    let paths = std::mem::take(&mut self.fx.drag_paths);
                    self.fx.copy_drag = false;
                    let accepted = dest.is_some();
                    if let Some(dest) = dest {
                        let (title, op) = if copy { ("Copying", Op::Copy) } else { ("Moving", Op::Move) };
                        self.start_job(title, JobKind::Transfer(op), paths, dest);
                    }
                    dnd::performed(ctx, session, accepted);
                } else {
                    let all = std::mem::take(&mut self.dnd.held);
                    let (mine, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|(r, ..)| *r == session);
                    self.dnd.held = rest;
                    // the copy runs as a job with progress; the drop is accepted once the job is started
                    let accepted = match dest.filter(|d| d.is_dir()) {
                        Some(dir) => self.start_receive("Copying", mine.into_iter().map(|(_, name, file)| (name, file)).collect(), dir),
                        None => false,
                    };
                    dnd::performed(ctx, session, accepted);
                    self.drag_out_foreign_over(session);
                }
                self.revision += 1;
                true
            }
        }
    }

    /// The other app's drag left this window (or ended): its held files go, and the hover state with it.
    fn drag_out_foreign_over(&mut self, session: u32) {
        if self.dnd.foreign == Some(session) {
            self.dnd.foreign = None;
            self.dnd.foreign_items = 0;
            self.dnd.held.retain(|(r, ..)| *r != session);
            self.dragging = false;
        }
    }

    /// `DragEnded`: the session of this window is over (the grants go back), or another app's drag is.
    pub(crate) fn drag_out_ended(&mut self, ctx: &Ctx, session: u32) -> bool {
        if self.dnd.session == Some(session) {
            self.dnd.session = None;
            for grant in std::mem::take(&mut self.dnd.session_grants) {
                ctx.release_document(grant);
            }
            if session_refused(std::mem::take(&mut self.dnd.heard), self.dragging) {
                // the shell refused the session (or ended it before the pointer moved): the pointer is still this
                // window's, so the drag goes on inside it and the release drops on the folder under it
                return true;
            }
            self.dragging = false;
            self.down = None;
            self.drop_target = None;
            self.spring_hover(None);
            self.fx.drag_paths.clear();
            self.fx.copy_drag = false;
            self.revision += 1;
            return true;
        }
        if self.dnd.foreign == Some(session) {
            self.drop_target = None;
            self.spring_hover(None);
            self.drag_out_foreign_over(session);
            self.revision += 1;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::dnd::dragop;

    #[test]
    fn a_drag_inside_the_window_moves_and_option_copies() {
        let both = dragop::COPY | dragop::MOVE;
        assert!(!own_drop_copies(false, both));
        assert!(own_drop_copies(true, both), "Option seen by the window");
        assert!(own_drop_copies(false, dragop::COPY), "Option narrowed the mask while the shell owns the pointer");
        assert_eq!(drop_reply(true, true, false), dragop::MOVE);
        assert_eq!(drop_reply(true, true, true), dragop::COPY);
        assert_eq!(drop_reply(true, false, false), dragop::COPY, "another app's files are copied");
        assert_eq!(drop_reply(false, true, false), 0, "not over a folder");
        assert_eq!(drop_reply(false, false, true), 0);
    }

    #[test]
    fn a_session_ended_before_any_dest_event_leaves_the_drag_in_the_window() {
        assert!(session_refused(false, true), "refused at once: the drag goes on in the window");
        assert!(!session_refused(true, true), "the shell owned the pointer: the drag is over");
        assert!(!session_refused(false, false), "nothing is being dragged any more");
    }
}
