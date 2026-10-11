//! Multi-item operations: copy and move with conflict resolution (replace,
//! keep both, skip) and progress, batch rename with numbering, and batch
//! delete to Recently Deleted. Each takes a callback that can cancel.

use super::{fs, trash};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Replace,
    KeepBoth,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

/// How far a batch has got.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub items_done: usize,
    pub items_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

impl Progress {
    /// 0.0..=1.0, by bytes when there are any, else by items.
    pub fn fraction(&self) -> f32 {
        if self.bytes_total > 0 {
            (self.bytes_done as f32 / self.bytes_total as f32).clamp(0.0, 1.0)
        } else if self.items_total > 0 {
            (self.items_done as f32 / self.items_total as f32).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// The outcome of a batch.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// where each processed source ended up
    pub done: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub cancelled: bool,
    /// (source, where it ended up) for each item that moved or was trashed, for Undo.
    pub moved: Vec<(PathBuf, PathBuf)>,
    /// The batch stopped early on an error that every later item would hit too (a full disk).
    pub stopped: bool,
}

/// What to do about a collision: the choice, and whether it applies to the
/// rest of the batch. None cancels the whole batch.
pub type Answer = Option<(Resolution, bool)>;

/// Sources whose name already exists in `dest` (the conflicts to ask about).
pub fn conflicts(sources: &[PathBuf], dest: &Path) -> Vec<usize> {
    sources.iter().enumerate().filter(|(_, s)| s.file_name().is_some_and(|n| dest.join(n).symlink_metadata().is_ok())).map(|(i, _)| i).collect()
}

/// (bytes, items) under `path`, links not followed; the work a batch has to do.
pub fn weigh(path: &Path) -> (u64, usize) {
    let Ok(md) = std::fs::symlink_metadata(path) else { return (0, 0) };
    if md.is_dir() {
        let (mut b, mut n) = (0, 1);
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                let (cb, cn) = weigh(&e.path());
                b += cb;
                n += cn;
            }
        }
        (b, n)
    } else {
        (md.len(), 1)
    }
}

struct Run<'a> {
    progress: Progress,
    tick: &'a mut dyn FnMut(&Progress) -> bool,
}

impl Run<'_> {
    fn report(&mut self) -> io::Result<()> {
        if (self.tick)(&self.progress) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"))
        }
    }
}

/// The hidden name a file is written under until it is whole.
fn part_name(dst: &Path) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dst.with_file_name(format!(".{name}.sbpart"))
}

/// Whether `e` is a full disk or quota, which every later item would hit as well.
pub fn is_disk_full(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(28) | Some(122))
}

/// Writes to a temporary name and renames it into place, so `dst` is never half a file.
fn copy_file(src: &Path, dst: &Path, run: &mut Run) -> io::Result<()> {
    let part = part_name(dst);
    let result = copy_file_to(src, &part, run).and_then(|()| std::fs::rename(&part, dst));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn copy_file_to(src: &Path, dst: &Path, run: &mut Run) -> io::Result<()> {
    let mut r = std::fs::File::open(src)?;
    let mut w = std::fs::File::create(dst)?;
    let mut buf = vec![0u8; 256 * 1024];
    let result: io::Result<()> = (|| loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        w.write_all(&buf[..n])?;
        run.progress.bytes_done += n as u64;
        run.report()?;
    })();
    result?;
    w.sync_all()
}

/// Whether `copy` holds what `original` does (same kinds, sizes and names), checked before an original is deleted.
pub fn verify_copy(original: &Path, copy: &Path) -> io::Result<()> {
    let (a, b) = (std::fs::symlink_metadata(original)?, std::fs::symlink_metadata(copy)?);
    let bad = || io::Error::other(format!("The copy of \u{201c}{}\u{201d} does not match the original", original.display()));
    if a.file_type().is_symlink() || b.file_type().is_symlink() {
        return if a.file_type().is_symlink() && b.file_type().is_symlink() && std::fs::read_link(original)? == std::fs::read_link(copy)? { Ok(()) } else { Err(bad()) };
    }
    if a.is_dir() != b.is_dir() {
        return Err(bad());
    }
    if a.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(original)?.flatten().map(|e| e.file_name()).collect();
        names.sort();
        let mut copied: Vec<_> = std::fs::read_dir(copy)?.flatten().map(|e| e.file_name()).collect();
        copied.sort();
        if names != copied {
            return Err(bad());
        }
        for n in names {
            verify_copy(&original.join(&n), &copy.join(&n))?;
        }
        Ok(())
    } else if a.len() == b.len() {
        Ok(())
    } else {
        Err(bad())
    }
}

fn copy_tree(src: &Path, dst: &Path, run: &mut Run) -> io::Result<()> {
    let md = std::fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        // a link is copied as a link, so a move across file systems loses nothing
        return std::os::unix::fs::symlink(std::fs::read_link(src)?, dst);
    }
    if md.is_dir() {
        std::fs::create_dir(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()), run)?;
        }
        Ok(())
    } else {
        copy_file(src, dst, run)
    }
}

/// Copies a file or folder tree to `dst` without progress; a failed copy
/// leaves no partial `dst` behind.
pub fn copy_path(src: &Path, dst: &Path) -> io::Result<()> {
    let mut tick = |_: &Progress| true;
    let mut run = Run { progress: Progress::default(), tick: &mut tick };
    copy_tree(src, dst, &mut run).inspect_err(|_| {
        let _ = fs::delete(dst);
    })
}

/// Files-style wording for the errors people hit.
pub fn friendly(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "You do not have permission".into(),
        io::ErrorKind::NotFound => "The item no longer exists".into(),
        _ if is_disk_full(e) => "The disk is full".into(),
        _ => e.to_string(),
    }
}

fn is_cancel(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted
}

/// Copies or moves `sources` into `dest`. On a name collision `ask` is
/// consulted (once, or once per item unless the answer applies to all).
/// `tick` is called as bytes move and returns false to cancel; a half-copied
/// item is removed. Items into their own folder are kept both (copy) or
/// left alone (move); a folder into itself fails.
pub fn transfer(op: Op, sources: &[PathBuf], dest: &Path, ask: &mut dyn FnMut(&Path) -> Answer, tick: &mut dyn FnMut(&Progress) -> bool) -> Report {
    let mut report = Report::default();
    let weights: Vec<(u64, usize)> = sources.iter().map(|s| weigh(s)).collect();
    let mut run = Run { progress: Progress { items_total: sources.len(), bytes_total: weights.iter().map(|w| w.0).sum(), ..Default::default() }, tick };
    let mut sticky: Option<Resolution> = None;
    for (src, w) in sources.iter().zip(&weights) {
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            report.failed.push((src.clone(), "Invalid name".into()));
            continue;
        };
        run.progress.current = name.clone();
        if run.report().is_err() {
            report.cancelled = true;
            return report;
        }
        let target = dest.join(&name);
        let same_place = src.parent() == Some(dest);
        if op == Op::Move && same_place {
            report.skipped.push(src.clone());
            run.progress.items_done += 1;
            run.progress.bytes_done += w.0;
            continue;
        }
        if dest.starts_with(src) && std::fs::symlink_metadata(src).is_ok_and(|m| m.is_dir()) {
            report.failed.push((src.clone(), "Cannot put a folder into itself".into()));
            continue;
        }
        let dst = if target.symlink_metadata().is_ok() {
            let choice = match sticky {
                Some(c) => c,
                None => match ask(&target) {
                    None => {
                        report.cancelled = true;
                        return report;
                    }
                    Some((c, all)) => {
                        if all {
                            sticky = Some(c);
                        }
                        c
                    }
                },
            };
            match choice {
                Resolution::Skip => {
                    report.skipped.push(src.clone());
                    run.progress.items_done += 1;
                    run.progress.bytes_done += w.0;
                    continue;
                }
                // replacing a file with itself would destroy it
                Resolution::Replace if same_place => fs::unique_name(dest, &name),
                Resolution::KeepBoth => fs::unique_name(dest, &name),
                Resolution::Replace => {
                    if let Err(e) = fs::delete(&target) {
                        report.failed.push((src.clone(), e.to_string()));
                        continue;
                    }
                    target
                }
            }
        } else {
            target
        };
        let before = run.progress.bytes_done;
        let result = match op {
            Op::Copy => copy_tree(src, &dst, &mut run).inspect_err(|_| {
                let _ = fs::delete(&dst);
            }),
            Op::Move => match std::fs::rename(src, &dst) {
                Ok(()) => {
                    run.progress.bytes_done += w.0;
                    Ok(())
                }
                Err(e) if e.raw_os_error() == Some(18) => copy_tree(src, &dst, &mut run).inspect_err(|_| {
                    let _ = fs::delete(&dst);
                }),
                Err(e) => Err(e),
            },
        };
        match result {
            Ok(()) => {
                if op == Op::Move && src.symlink_metadata().is_ok() {
                    // cross-device move: verify the copy, and only then remove the original
                    if let Err(e) = verify_copy(src, &dst) {
                        let _ = fs::delete(&dst);
                        report.failed.push((src.clone(), e.to_string()));
                        continue;
                    }
                    if let Err(e) = fs::delete(src) {
                        report.failed.push((src.clone(), e.to_string()));
                        continue;
                    }
                }
                run.progress.bytes_done = before + w.0;
                run.progress.items_done += 1;
                if op == Op::Move {
                    report.moved.push((src.clone(), dst.clone()));
                }
                report.done.push(dst);
            }
            Err(e) if is_cancel(&e) => {
                report.cancelled = true;
                return report;
            }
            Err(e) if is_disk_full(&e) => {
                report.failed.push((src.clone(), "The disk is full".into()));
                report.stopped = true;
                return report;
            }
            Err(e) => report.failed.push((src.clone(), friendly(&e))),
        }
    }
    run.progress.current.clear();
    let _ = run.report();
    report
}

/// Deletes each path into Recently Deleted (for good when already inside
/// `bin`). `tick` returns false to stop between items.
pub fn delete_all(paths: &[PathBuf], bin: &Path, tick: &mut dyn FnMut(&Progress) -> bool) -> Report {
    let mut report = Report::default();
    let mut p = Progress { items_total: paths.len(), ..Default::default() };
    for path in paths {
        p.current = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !tick(&p) {
            report.cancelled = true;
            return report;
        }
        match trash::delete_item(path, bin) {
            Ok(()) => report.done.push(path.clone()),
            Err(e) => report.failed.push((path.clone(), e.to_string())),
        }
        p.items_done += 1;
    }
    p.current.clear();
    tick(&p);
    report
}

/// The numbered names a batch rename gives `originals`: "Base 1.ext",
/// "Base 2.ext"… counting from `start`, each keeping its own extension.
/// Numbers are zero padded to the width of the largest.
pub fn numbered_names(originals: &[String], base: &str, start: u32) -> Vec<String> {
    let last = start as usize + originals.len().saturating_sub(1);
    let width = last.to_string().len();
    originals
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ext = name.rsplit_once('.').filter(|_| fs::extension(name).is_some()).map(|(_, e)| format!(".{e}")).unwrap_or_default();
            format!("{} {:0width$}{ext}", base.trim(), start as usize + i, width = width)
        })
        .collect()
}

/// Plans a batch rename as (from, to) pairs in one folder. Fails when the
/// base is not a valid name or a target collides with a file outside the
/// selection.
pub fn plan_rename(paths: &[PathBuf], base: &str, start: u32) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    if !fs::valid_name(base) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid name"));
    }
    let names: Vec<String> = paths.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    let news = numbered_names(&names, base, start);
    let mut plan = vec![];
    for (p, n) in paths.iter().zip(news) {
        let to = p.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid name"))?.join(n);
        if to.symlink_metadata().is_ok() && !paths.contains(&to) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "A file with that name already exists"));
        }
        plan.push((p.clone(), to));
    }
    Ok(plan)
}

/// Runs a rename plan through temporary names so targets that are also
/// sources (a shifted numbering) never clobber each other.
pub fn rename_all(plan: &[(PathBuf, PathBuf)]) -> io::Result<Vec<PathBuf>> {
    let changing: Vec<&(PathBuf, PathBuf)> = plan.iter().filter(|(a, b)| a != b).collect();
    let mut staged = vec![];
    for (i, (from, _)) in changing.iter().enumerate() {
        let tmp = from.with_file_name(format!(".sbren-{}-{i}", std::process::id()));
        std::fs::rename(from, &tmp)?;
        staged.push(tmp);
    }
    for (tmp, (_, to)) in staged.iter().zip(&changing) {
        std::fs::rename(tmp, to)?;
    }
    Ok(plan.iter().map(|(_, to)| to.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let d = fs::tests::temp(tag);
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        (d, a, b)
    }

    fn never(_: &Path) -> Answer {
        panic!("no conflict expected")
    }

    #[test]
    fn copy_multiple_with_progress() {
        let (_d, a, b) = setup("batch-copy");
        std::fs::write(a.join("x.txt"), vec![1u8; 300_000]).unwrap();
        std::fs::create_dir_all(a.join("dir/in")).unwrap();
        std::fs::write(a.join("dir/in/y.txt"), "yy").unwrap();
        let mut last = Progress::default();
        let r = transfer(Op::Copy, &[a.join("x.txt"), a.join("dir")], &b, &mut never, &mut |p| {
            last = p.clone();
            true
        });
        assert_eq!(r.done, [b.join("x.txt"), b.join("dir")]);
        assert!(a.join("x.txt").exists());
        assert_eq!(std::fs::read(b.join("dir/in/y.txt")).unwrap(), b"yy");
        assert_eq!((last.items_done, last.items_total), (2, 2));
        assert_eq!(last.bytes_done, 300_002);
        assert!((last.fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn move_removes_sources_and_ignores_same_folder() {
        let (_d, a, b) = setup("batch-move");
        std::fs::write(a.join("m.txt"), "m").unwrap();
        std::fs::write(a.join("stay.txt"), "s").unwrap();
        let r = transfer(Op::Move, &[a.join("m.txt")], &b, &mut never, &mut |_| true);
        assert_eq!(r.done, [b.join("m.txt")]);
        assert!(!a.join("m.txt").exists());
        let r = transfer(Op::Move, &[a.join("stay.txt")], &a, &mut never, &mut |_| true);
        assert_eq!(r.skipped, [a.join("stay.txt")]);
        assert!(a.join("stay.txt").exists());
    }

    #[test]
    fn conflict_replace_keep_both_skip() {
        let (_d, a, b) = setup("batch-conflict");
        for n in ["one", "two", "three"] {
            std::fs::write(a.join(n), format!("new {n}")).unwrap();
            std::fs::write(b.join(n), format!("old {n}")).unwrap();
        }
        let srcs = [a.join("one"), a.join("two"), a.join("three")];
        assert_eq!(conflicts(&srcs, &b), [0, 1, 2]);
        let mut answers = vec![(Resolution::Replace, false), (Resolution::KeepBoth, false), (Resolution::Skip, false)].into_iter();
        let r = transfer(Op::Copy, &srcs, &b, &mut |_| answers.next(), &mut |_| true);
        assert_eq!(std::fs::read_to_string(b.join("one")).unwrap(), "new one");
        assert_eq!(std::fs::read_to_string(b.join("two")).unwrap(), "old two");
        assert_eq!(std::fs::read_to_string(b.join("two 2")).unwrap(), "new two");
        assert_eq!(std::fs::read_to_string(b.join("three")).unwrap(), "old three");
        assert_eq!(r.skipped, [a.join("three")]);
        assert_eq!(r.done.len(), 2);
    }

    #[test]
    fn apply_to_all_asks_once() {
        let (_d, a, b) = setup("batch-all");
        for n in ["p", "q"] {
            std::fs::write(a.join(n), "new").unwrap();
            std::fs::write(b.join(n), "old").unwrap();
        }
        let mut asked = 0;
        let r = transfer(Op::Move, &[a.join("p"), a.join("q")], &b, &mut |_| {
            asked += 1;
            Some((Resolution::Replace, true))
        }, &mut |_| true);
        assert_eq!(asked, 1);
        assert_eq!(r.done.len(), 2);
        assert_eq!(std::fs::read_to_string(b.join("q")).unwrap(), "new");
        assert!(!a.join("q").exists());
    }

    #[test]
    fn answering_none_cancels_and_replace_in_place_keeps_both() {
        let (_d, a, b) = setup("batch-cancel-ask");
        std::fs::write(a.join("f"), "1").unwrap();
        std::fs::write(b.join("f"), "2").unwrap();
        let r = transfer(Op::Copy, &[a.join("f")], &b, &mut |_| None, &mut |_| true);
        assert!(r.cancelled && r.done.is_empty());
        let r = transfer(Op::Copy, &[a.join("f")], &a, &mut |_| Some((Resolution::Replace, false)), &mut |_| true);
        assert_eq!(r.done, [a.join("f 2")]);
        assert_eq!(std::fs::read_to_string(a.join("f")).unwrap(), "1");
    }

    #[test]
    fn cancel_mid_copy_removes_partial() {
        let (_d, a, b) = setup("batch-cancel");
        std::fs::write(a.join("big"), vec![7u8; 2_000_000]).unwrap();
        let mut calls = 0;
        let r = transfer(Op::Copy, &[a.join("big")], &b, &mut never, &mut |_| {
            calls += 1;
            calls < 3
        });
        assert!(r.cancelled);
        assert!(!b.join("big").exists());
        assert!(a.join("big").exists());
    }

    #[test]
    fn folder_into_itself_fails() {
        let (_d, a, _b) = setup("batch-self");
        std::fs::create_dir(a.join("in")).unwrap();
        let r = transfer(Op::Copy, &[a.clone()], &a.join("in"), &mut never, &mut |_| true);
        assert_eq!(r.failed.len(), 1);
    }

    #[test]
    fn batch_delete_goes_to_recently_deleted() {
        let (d, a, _b) = setup("batch-delete");
        std::fs::write(a.join("g1"), "1").unwrap();
        std::fs::write(a.join("g2"), "2").unwrap();
        let bin = trash::dir(&d);
        let mut seen = vec![];
        let r = delete_all(&[a.join("g1"), a.join("g2")], &bin, &mut |p| {
            seen.push(p.items_done);
            true
        });
        assert_eq!(r.done.len(), 2);
        assert!(!a.join("g1").exists() && !a.join("g2").exists());
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 2);
        assert_eq!(seen, [0, 1, 2]);
        let r = delete_all(&[a.join("g1")], &bin, &mut |_| false);
        assert!(r.cancelled);
    }

    #[test]
    fn numbering_pads_and_keeps_extensions() {
        let names: Vec<String> = ["a.jpg", "b.png", "noext"].iter().map(|s| s.to_string()).collect();
        assert_eq!(numbered_names(&names, "Trip", 1), ["Trip 1.jpg", "Trip 2.png", "Trip 3"]);
        let many: Vec<String> = (0..12).map(|i| format!("f{i}.txt")).collect();
        let n = numbered_names(&many, "Doc", 1);
        assert_eq!((n[0].as_str(), n[11].as_str()), ("Doc 01.txt", "Doc 12.txt"));
    }

    #[test]
    fn rename_plan_and_shifted_numbering() {
        let (_d, a, _b) = setup("batch-rename");
        for n in ["Pic 2.txt", "x.txt", "y.txt"] {
            std::fs::write(a.join(n), n).unwrap();
        }
        // "Pic 2.txt" is both a source and a target: staging keeps it safe
        let sel = [a.join("x.txt"), a.join("Pic 2.txt")];
        let plan = plan_rename(&sel, "Pic", 1).unwrap();
        assert_eq!(plan[0].1, a.join("Pic 1.txt"));
        assert_eq!(plan[1].1, a.join("Pic 2.txt"));
        let out = rename_all(&plan).unwrap();
        assert_eq!(std::fs::read_to_string(&out[0]).unwrap(), "x.txt");
        assert_eq!(std::fs::read_to_string(&out[1]).unwrap(), "Pic 2.txt");
        assert!(!a.join("x.txt").exists());
        // a target held by an unselected file is refused
        assert!(plan_rename(&[a.join("y.txt")], "Pic", 2).is_err());
        assert!(plan_rename(&[a.join("y.txt")], "a/b", 1).is_err());
    }

    #[test]
    fn rename_swaps_through_staging() {
        let (_d, a, _b) = setup("batch-swap");
        std::fs::write(a.join("one.txt"), "1").unwrap();
        std::fs::write(a.join("two.txt"), "2").unwrap();
        let plan = vec![(a.join("one.txt"), a.join("two.txt")), (a.join("two.txt"), a.join("one.txt"))];
        rename_all(&plan).unwrap();
        assert_eq!(std::fs::read_to_string(a.join("one.txt")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(a.join("two.txt")).unwrap(), "1");
    }

    #[test]
    fn copy_leaves_no_part_file_and_failure_leaves_no_target() {
        let (_d, a, b) = setup("batch-part");
        std::fs::write(a.join("f"), "data").unwrap();
        copy_file(&a.join("f"), &b.join("f"), &mut Run { progress: Progress::default(), tick: &mut |_| true }).unwrap();
        assert_eq!(std::fs::read_dir(&b).unwrap().count(), 1);
        let r = copy_file(&a.join("missing"), &b.join("g"), &mut Run { progress: Progress::default(), tick: &mut |_| true });
        assert!(r.is_err() && !b.join("g").exists() && !part_name(&b.join("g")).exists());
    }

    #[test]
    fn verify_copy_catches_a_short_copy() {
        let (_d, a, b) = setup("batch-verify");
        std::fs::write(a.join("f"), "12345").unwrap();
        std::fs::write(b.join("f"), "123").unwrap();
        assert!(verify_copy(&a.join("f"), &b.join("f")).is_err());
        std::fs::write(b.join("f"), "54321").unwrap();
        assert!(verify_copy(&a.join("f"), &b.join("f")).is_ok());
        std::fs::create_dir_all(a.join("d/e")).unwrap();
        std::fs::create_dir_all(b.join("d")).unwrap();
        assert!(verify_copy(&a.join("d"), &b.join("d")).is_err());
    }

    #[test]
    fn broken_symlink_is_copied_and_collides() {
        let (_d, a, b) = setup("batch-dangling");
        std::os::unix::fs::symlink("nowhere", a.join("ln")).unwrap();
        let r = transfer(Op::Copy, &[a.join("ln")], &b, &mut never, &mut |_| true);
        assert_eq!(r.done, [b.join("ln")]);
        assert_eq!(std::fs::read_link(b.join("ln")).unwrap(), Path::new("nowhere"));
        assert_eq!(conflicts(&[a.join("ln")], &b), [0]);
        let r = transfer(Op::Copy, &[a.join("ln")], &b, &mut |_| Some((Resolution::KeepBoth, false)), &mut |_| true);
        assert_eq!(r.done, [b.join("ln 2")]);
    }

    #[test]
    fn missing_source_fails_without_panic_and_move_records_pairs() {
        let (_d, a, b) = setup("batch-missing");
        let r = transfer(Op::Move, &[a.join("gone")], &b, &mut never, &mut |_| true);
        assert_eq!(r.failed.len(), 1);
        std::fs::write(a.join("m"), "m").unwrap();
        let r = transfer(Op::Move, &[a.join("m")], &b, &mut never, &mut |_| true);
        assert_eq!(r.moved, [(a.join("m"), b.join("m"))]);
    }

    #[test]
    fn disk_full_and_permission_wording() {
        assert!(is_disk_full(&io::Error::from_raw_os_error(28)));
        assert_eq!(friendly(&io::Error::from_raw_os_error(28)), "The disk is full");
        assert_eq!(friendly(&io::Error::from(io::ErrorKind::PermissionDenied)), "You do not have permission");
    }
}
