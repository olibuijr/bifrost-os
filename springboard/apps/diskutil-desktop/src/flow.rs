//! The open and save panels. Open Disk Image, New Image from Folder, Convert Image and Resize Image ask the shell for a
//! file in its open panel, and the new image commands ask for a place in its save panel. The answer arrives later as a
//! document event, so the window keeps what it is waiting for (`Flow`) and what it wants to ask next (`Want`). A panel
//! needs the app context, which the logic does not have, so the logic only records the want and `flush` (called by the
//! app after each event, menu command and tick) sends it. A save is a two-step answer: the panel hands over a file to
//! write (`SaveTo`), the job writes it, and `flush` then commits it, or throws it away when the job failed.

use crate::images::{self, BlankSpec, ImageFormat};
use crate::window::{DiskUtility, Job};
use bifrost_app::proto::DocErrorCode;
use bifrost_app::{Ctx, Document, DocumentEvent, OpenOptions};
use std::fs::File;

/// What a file chosen in an open panel is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenThen {
    /// Bind the image to a loop device and mount it.
    Attach,
    /// Ask which format to convert the image to.
    Convert,
    /// Ask for the image's new size.
    Resize,
    /// Ask where to save an ISO image of the folder.
    Folder,
}

/// What the file a save panel hands over is written with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOp {
    Blank(BlankSpec),
    Folder { folder: String, volname: String },
    Convert { src: String, format: ImageFormat },
}

impl SaveOp {
    /// The progress word shown while the file is written.
    pub fn verb(&self) -> &'static str {
        match self {
            SaveOp::Blank(_) | SaveOp::Folder { .. } => "Creating",
            SaveOp::Convert { .. } => "Converting",
        }
    }
}

/// A panel the window wants shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Want {
    Open { title: &'static str, filters: Vec<&'static str>, directories: bool, then: OpenThen },
    Save { suggested: String, filters: Vec<&'static str>, op: SaveOp },
}

/// A panel that is open, by the request id the answer will carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    Open(u32, OpenThen),
    Save(u32, SaveOp),
}

impl Flow {
    fn request(&self) -> u32 {
        match self {
            Flow::Open(request, _) | Flow::Save(request, _) => *request,
        }
    }
}

/// The sentence for a panel that did not produce a file.
pub fn doc_error_sentence(code: DocErrorCode) -> String {
    match code {
        DocErrorCode::Cancelled => String::new(),
        DocErrorCode::Denied => "Disk Utility may not open that file.".to_string(),
        DocErrorCode::NotFound => "The file could not be found.".to_string(),
        DocErrorCode::Stale => "The file changed since it was chosen. Choose it again.".to_string(),
        DocErrorCode::Revoked => "Access to the file was taken back.".to_string(),
        DocErrorCode::Unsupported => "Disk Utility has no permission to open or save files.".to_string(),
    }
}

impl DiskUtility {
    /// Whether anything is open or running that a new command must wait for.
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.sheet.is_some() || self.form.is_some() || self.flow.is_some() || self.want.is_some()
    }

    /// Records that an open panel is wanted. False when something else is going on.
    fn want_open(&mut self, title: &'static str, filters: &[&'static str], directories: bool, then: OpenThen) -> bool {
        if self.busy() {
            return false;
        }
        self.error = None;
        self.notice = None;
        self.want = Some(Want::Open { title, filters: filters.to_vec(), directories, then });
        true
    }

    /// File > Open Disk Image: choose an image to attach.
    pub fn open_image(&mut self) -> bool {
        self.want_open("Open Disk Image", &["iso", "img", "raw"], false, OpenThen::Attach)
    }

    /// File > New Image > Image from Folder: choose the folder.
    pub fn new_image_from_folder(&mut self) -> bool {
        self.want_open("Choose a Folder", &[], true, OpenThen::Folder)
    }

    /// Image > Convert Image: choose the image to convert.
    pub fn convert_image(&mut self) -> bool {
        self.want_open("Choose an Image to Convert", &["img", "raw", "iso", "qcow2", "vmdk", "vhdx", "vdi"], false, OpenThen::Convert)
    }

    /// Image > Resize Image: choose the image to resize.
    pub fn resize_image(&mut self) -> bool {
        self.want_open("Choose an Image to Resize", &["img", "raw"], false, OpenThen::Resize)
    }

    /// Sends the panel the window wants, commits or throws away a finished save, and tells the shell whether a text
    /// field has the keyboard. Called after every event, menu command and tick.
    pub fn flush(&mut self, ctx: &Ctx) {
        if let Some(want) = self.want.take() {
            match want {
                Want::Open { title, filters, directories, then } => {
                    let options = OpenOptions {
                        title: title.to_string(),
                        filters: filters.iter().map(|f| f.to_string()).collect(),
                        files: !directories,
                        directories,
                        ..OpenOptions::default()
                    };
                    let request = ctx.open_panel(&options);
                    self.flow = Some(Flow::Open(request, then));
                }
                Want::Save { suggested, filters, op } => {
                    let request = ctx.save_document(&suggested, &filters);
                    self.flow = Some(Flow::Save(request, op));
                }
            }
        }
        if let Some((request, written)) = self.finished_save.take() {
            if written {
                ctx.commit_document(request);
            } else {
                ctx.abort_document(request);
            }
        }
        ctx.wants_keyboard(self.wants_keyboard());
    }

    /// The shell's answer to a panel. True when the window changed.
    pub fn on_document(&mut self, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::Opened(doc) => self.opened(doc),
            DocumentEvent::SaveTo(doc) => {
                let Document { request, name, file, .. } = doc;
                self.save_job(request, name, file)
            }
            DocumentEvent::Saved(doc) => {
                self.notice = Some(format!("Saved {}.", doc.name));
                true
            }
            DocumentEvent::Failed { request, code } => self.failed(request, code),
            DocumentEvent::OpenedMany { .. } => false,
        }
    }

    /// An open panel produced a file or a folder.
    fn opened(&mut self, doc: Document) -> bool {
        let size = doc.file.metadata().map(|m| m.len()).unwrap_or(doc.size);
        let path = images::file_path(&doc.file);
        let head = images::read_head(&doc.file, 1084);
        self.opened_path(doc.request, path, size, &head)
    }

    /// What follows a chosen file: `path` is where it is (or why that is unknown), `size` its length and `head` its
    /// first bytes.
    pub fn opened_path(&mut self, request: u32, path: Result<String, String>, size: u64, head: &[u8]) -> bool {
        let then = match self.flow.take() {
            Some(Flow::Open(wanted, then)) if wanted == request => then,
            other => {
                self.flow = other;
                return false;
            }
        };
        let path = match path {
            Ok(path) => path,
            Err(sentence) => {
                self.error = Some(sentence);
                return true;
            }
        };
        match then {
            OpenThen::Attach => match images::attach_plan(&path) {
                Ok(plan) => self.start_plan("Attaching", images::base_name(&path).to_string(), plan, false),
                Err(sentence) => self.error = Some(sentence),
            },
            OpenThen::Convert => self.open_convert_form(path),
            OpenThen::Resize => self.open_resize_form(path, size, images::is_ext_image(head)),
            OpenThen::Folder => {
                let suggested = images::suggested_file(images::base_name(&path), "iso");
                let volname = images::iso_volume_name(&path);
                self.want = Some(Want::Save { suggested, filters: vec!["iso"], op: SaveOp::Folder { folder: path, volname } });
            }
        }
        true
    }

    /// A save panel handed over `file` to write. The job writes it; `flush` commits it afterwards.
    pub fn save_job(&mut self, request: u32, name: String, file: File) -> bool {
        let op = match self.flow.take() {
            Some(Flow::Save(wanted, op)) if wanted == request => op,
            other => {
                self.flow = other;
                self.finished_save = Some((request, false));
                return false;
            }
        };
        if self.job.is_some() {
            self.finished_save = Some((request, false));
            self.error = Some("Wait for the running operation to finish.".to_string());
            return true;
        }
        let verb = op.verb();
        let rx = match op {
            SaveOp::Blank(spec) => images::spawn(move || images::write_blank(file, spec)),
            SaveOp::Folder { folder, volname } => images::spawn(move || images::write_folder_image(file, &folder, &volname)),
            SaveOp::Convert { src, format } => images::spawn(move || images::write_converted(file, &src, format)),
        };
        self.error = None;
        self.notice = None;
        self.job = Some(Job { verb, name, rx, commit: Some(request) });
        true
    }

    /// A panel produced no file: the user cancelled, or the shell refused.
    fn failed(&mut self, request: u32, code: DocErrorCode) -> bool {
        if self.flow.as_ref().map(Flow::request) != Some(request) {
            return false;
        }
        self.flow = None;
        if code != DocErrorCode::Cancelled {
            self.error = Some(doc_error_sentence(code));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::BlankSpec;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("bifrost-diskutil-{name}-{}", std::process::id()))
    }

    fn wait(w: &mut DiskUtility) {
        for _ in 0..500 {
            if w.poll(1.0) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the job did not finish");
    }

    #[test]
    fn a_panel_is_wanted_only_when_nothing_else_is_going_on() {
        let mut w = DiskUtility::new();
        assert!(w.open_image());
        assert_eq!(
            w.want,
            Some(Want::Open { title: "Open Disk Image", filters: vec!["iso", "img", "raw"], directories: false, then: OpenThen::Attach })
        );
        assert!(!w.resize_image(), "the first panel is still wanted");
        w.want = None;
        w.flow = Some(Flow::Open(3, OpenThen::Attach));
        assert!(!w.convert_image());
        assert!(w.busy());
    }

    #[test]
    fn a_folder_panel_chooses_directories_not_files() {
        let mut w = DiskUtility::new();
        assert!(w.new_image_from_folder());
        match w.want.take() {
            Some(Want::Open { directories, then, .. }) => assert!(directories && then == OpenThen::Folder),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_image_that_was_chosen_is_attached_through_the_image_tool() {
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Open(7, OpenThen::Attach));
        assert!(w.opened_path(7, Ok("/home/me/disc.iso".into()), 700, &[]));
        let job = w.job.as_ref().expect("an attach job");
        assert_eq!((job.verb, job.name.as_str()), ("Attaching", "disc.iso"));
        assert!(w.flow.is_none() && job.commit.is_none());
    }

    #[test]
    fn an_answer_for_another_request_is_left_alone() {
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Open(7, OpenThen::Attach));
        assert!(!w.opened_path(8, Ok("/a.iso".into()), 1, &[]));
        assert_eq!(w.flow, Some(Flow::Open(7, OpenThen::Attach)));
        assert!(w.job.is_none());
    }

    #[test]
    fn a_path_that_cannot_be_read_becomes_the_footer_error() {
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Open(1, OpenThen::Attach));
        assert!(w.opened_path(1, Err("The location cannot be used.".into()), 1, &[]));
        assert_eq!(w.footer(), Some(("The location cannot be used.".to_string(), true)));
        assert!(w.job.is_none());
    }

    #[test]
    fn a_chosen_folder_asks_for_a_place_to_save_its_iso() {
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Open(2, OpenThen::Folder));
        assert!(w.opened_path(2, Ok("/home/me/My Photos".into()), 0, &[]));
        assert_eq!(
            w.want,
            Some(Want::Save {
                suggested: "My Photos.iso".into(),
                filters: vec!["iso"],
                op: SaveOp::Folder { folder: "/home/me/My Photos".into(), volname: "MY_PHOTOS".into() }
            })
        );
    }

    #[test]
    fn a_chosen_image_to_resize_opens_the_size_sheet_with_its_file_system_known() {
        let mut w = DiskUtility::new();
        let mut head = vec![0u8; 2048];
        head[1080] = 0x53;
        head[1081] = 0xEF;
        w.flow = Some(Flow::Open(4, OpenThen::Resize));
        assert!(w.opened_path(4, Ok("/home/me/a.img".into()), 64 << 20, &head));
        let open = w.form.as_ref().expect("the resize sheet");
        assert_eq!(open.form.value(0), "64m");
    }

    #[test]
    fn a_cancelled_panel_ends_the_flow_quietly_and_a_refusal_is_told() {
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Open(5, OpenThen::Attach));
        assert!(!w.failed(6, DocErrorCode::Denied), "not our request");
        assert!(w.failed(5, DocErrorCode::Cancelled));
        assert!(w.flow.is_none() && w.error.is_none());
        w.flow = Some(Flow::Save(9, SaveOp::Blank(BlankSpec { bytes: 512, fs: None, label: String::new() })));
        assert!(w.failed(9, DocErrorCode::Unsupported));
        assert!(w.error.as_deref().is_some_and(|e| e.contains("permission")));
        assert!(doc_error_sentence(DocErrorCode::Cancelled).is_empty());
    }

    #[test]
    fn a_save_with_no_matching_wait_is_thrown_away() {
        let path = temp("stray");
        let file = File::create(&path).unwrap();
        let mut w = DiskUtility::new();
        assert!(!w.save_job(11, "x.img".into(), file));
        assert_eq!(w.finished_save, Some((11, false)));
        assert!(w.job.is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_blank_image_is_sized_by_a_job_and_then_committed() {
        let path = temp("blank");
        let file = File::create(&path).unwrap();
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Save(12, SaveOp::Blank(BlankSpec { bytes: 1 << 20, fs: None, label: String::new() })));
        assert!(w.save_job(12, "blank.img".into(), file));
        let job = w.job.as_ref().expect("a save job");
        assert_eq!((job.verb, job.commit), ("Creating", Some(12)));
        wait(&mut w);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1 << 20);
        assert_eq!(w.finished_save, Some((12, true)));
        assert!(w.error.is_none() && w.job.is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_failed_job_aborts_the_save_instead_of_committing_it() {
        let path = temp("failed");
        let file = File::create(&path).unwrap();
        let mut w = DiskUtility::new();
        w.flow = Some(Flow::Save(13, SaveOp::Convert { src: "/nonexistent/in.img".into(), format: ImageFormat::Qcow2 }));
        assert!(w.save_job(13, "out.qcow2".into(), file));
        wait(&mut w);
        assert_eq!(w.finished_save, Some((13, false)));
        assert!(w.error.is_some());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_second_save_while_a_job_runs_is_refused() {
        let path = temp("busy");
        let file = File::create(&path).unwrap();
        let mut w = DiskUtility::new();
        w.job = Some(Job { verb: "Mounting", name: "sdb1".into(), rx: images::spawn(|| Ok(())), commit: None });
        w.flow = Some(Flow::Save(14, SaveOp::Blank(BlankSpec { bytes: 512, fs: None, label: String::new() })));
        assert!(w.save_job(14, "b.img".into(), file));
        assert_eq!(w.finished_save, Some((14, false)));
        assert!(w.error.is_some());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_operations_name_their_progress() {
        assert_eq!(SaveOp::Folder { folder: "/a".into(), volname: String::new() }.verb(), "Creating");
        assert_eq!(SaveOp::Convert { src: "/a".into(), format: ImageFormat::Vdi }.verb(), "Converting");
    }
}
