//! Printing the document as it is marked up. A PDF is edited on its worker (page order, turns, markup, as Export PDF
//! writes it), the edited copy is written to the app's data directory, and that copy is sent to the printer, so the
//! printout is what the window shows. A picture is printed as its marked up page. The worker runs the job, so the
//! window stays responsive while the file is written and sent.

use crate::model::Mark;
use crate::pdfjob::{self, Job, Pdf, Reply};
use crate::pdfmarks;
use bifrost_app::print::{PrintError, PrintOperation};
use std::path::{Path, PathBuf};

/// The status line after a job is sent to the printer.
pub const SENT: &str = "Sent to the printer";

/// The file a print of `path` is written to: `print/<name>.<ext>` under the app's data directory.
pub fn scratch_file(data_dir: &Path, path: &Path, ext: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
    data_dir.join("print").join(format!("{stem}.{ext}"))
}

/// The status line for a print error.
pub fn message(e: PrintError) -> String {
    match e {
        PrintError::NoCups => "The printing service is not running".into(),
        PrintError::NoPrinter => "No printer is set up".into(),
        PrintError::Failed(message) => format!("Printing failed: {message}"),
    }
}

/// Sends the file at `path` to the default printer. Returns the status line.
pub fn print_file(path: PathBuf) -> String {
    match PrintOperation::new(path).run() {
        Ok(_) => SENT.into(),
        Err(e) => message(e),
    }
}

/// A job that writes the edited PDF to `target` and prints that file. It replies with the status line.
pub fn pdf_job(source: PathBuf, target: PathBuf, order: Vec<usize>, turns: Vec<u8>, marks: Vec<Mark>) -> Job {
    pdfjob::run(move |pdf: &Pdf| {
        let status = match prepare(pdf, &source, &target, &order, &turns, &marks) {
            Ok(file) => print_file(file),
            Err(e) => format!("Printing failed: {e}"),
        };
        Reply::Printed(status)
    })
}

/// Writes the edited PDF to `target`, making its folder first.
fn prepare(pdf: &Pdf, source: &Path, target: &Path, order: &[usize], turns: &[u8], marks: &[Mark]) -> Result<PathBuf, String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    pdfmarks::write_edited(pdf, source, target, order, turns, marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_files_are_kept_in_the_data_directory_named_after_the_document() {
        assert_eq!(scratch_file(Path::new("/d"), Path::new("/a/b.pdf"), "pdf"), PathBuf::from("/d/print/b.pdf"));
        assert_eq!(scratch_file(Path::new("/d"), Path::new("/a/b.png"), "png"), PathBuf::from("/d/print/b.png"));
    }

    #[test]
    fn print_errors_read_as_status_lines() {
        assert_eq!(message(PrintError::NoCups), "The printing service is not running");
        assert_eq!(message(PrintError::NoPrinter), "No printer is set up");
        assert_eq!(message(PrintError::Failed("busy".into())), "Printing failed: busy");
    }
}
