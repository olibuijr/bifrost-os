//! Bounded, escaped, atomically written store. Format v2 keeps lists and
//! full reminders; the v1 date-only format still loads (all in one list).
use crate::model::{Data, ListDef, Reminder, MAX_NAME, MAX_NOTES, MAX_TITLE, MAX_URL};
use crate::repeat::Repeat;
use springboard::shell::prefs::{escape_field as escaped, unescape_field};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const MAX_ITEMS: usize = 10_000;
const MAX_LISTS: usize = 100;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const HEADER: &str = "springboard-reminders-v2";
const HEADER_V1: &str = "springboard-reminders-v1";
const DAY_RANGE: std::ops::RangeInclusive<i64> = -719_162..=2_932_896;

pub fn local_day(unix: i64, offset: i32) -> i64 {
    unix.saturating_add(offset as i64).div_euclid(86_400)
}

fn unescape(text: &str, max: usize, allow_empty: bool) -> Option<String> {
    let out = unescape_field(text, max)?;
    let blank = out.trim().is_empty();
    (!(blank && !allow_empty)
        && !out
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')))
    .then_some(out)
}

fn opt_num<T: std::str::FromStr>(field: &str) -> Result<Option<T>, ()> {
    if field == "-" {
        Ok(None)
    } else {
        field.parse().map(Some).map_err(|_| ())
    }
}

fn flag(field: &str) -> Option<bool> {
    match field {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn parse_item(fields: &[&str]) -> Option<Reminder> {
    // R id list parent due time done flag prio repeat early title notes url
    if fields.len() != 14 {
        return None;
    }
    let due: Option<i64> = opt_num(fields[4]).ok()?;
    if due.is_some_and(|d| !DAY_RANGE.contains(&d)) {
        return None;
    }
    let time: Option<u16> = opt_num(fields[5]).ok()?;
    if time.is_some_and(|t| t >= 1440) {
        return None;
    }
    let priority: u8 = fields[8].parse().ok().filter(|p| *p <= 3)?;
    let repeat = if fields[9] == "-" { None } else { Some(Repeat::decode(fields[9])?) };
    Some(Reminder {
        id: fields[1].parse().ok()?,
        list: fields[2].parse().ok()?,
        parent: opt_num(fields[3]).ok()?,
        due,
        time,
        completed: flag(fields[6])?,
        flagged: flag(fields[7])?,
        priority,
        repeat,
        early: opt_num(fields[10]).ok()?,
        title: unescape(fields[11], MAX_TITLE, false)?,
        notes: unescape(fields[12], MAX_NOTES, true)?,
        url: unescape(fields[13], MAX_URL, true)?,
    })
}

fn parse_v1(line: &str) -> Option<Reminder> {
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != 4 {
        return None;
    }
    let due: Option<i64> = opt_num(fields[1]).ok()?;
    if due.is_some_and(|d| !DAY_RANGE.contains(&d)) {
        return None;
    }
    let mut r = Reminder::new(fields[0].parse().ok()?, unescape(fields[3], MAX_TITLE, false)?);
    r.due = due;
    r.completed = flag(fields[2])?;
    Some(r)
}

pub fn parse(text: &str) -> Data {
    let mut lines = text.lines();
    let v1 = match lines.next() {
        Some(HEADER) => false,
        Some(HEADER_V1) => true,
        _ => return Data::default(),
    };
    let mut lists: Vec<ListDef> = vec![];
    let mut items: Vec<Reminder> = vec![];
    let mut list_ids = HashSet::new();
    let mut ids = HashSet::new();
    for line in lines.take(MAX_ITEMS + MAX_LISTS) {
        if v1 {
            if let Some(r) = parse_v1(line).filter(|r| ids.insert(r.id)) {
                items.push(r);
            }
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        match fields.first().copied() {
            Some("L") if fields.len() == 5 && lists.len() < MAX_LISTS => {
                let (Ok(id), Ok(color), Ok(icon)) = (fields[1].parse::<u64>(), fields[2].parse::<u8>(), fields[3].parse::<u8>()) else {
                    continue;
                };
                let Some(name) = unescape(fields[4], MAX_NAME, false) else { continue };
                if list_ids.insert(id) {
                    lists.push(ListDef { id, name, color, icon });
                }
            }
            Some("R") if items.len() < MAX_ITEMS => {
                if let Some(r) = parse_item(&fields).filter(|r| ids.insert(r.id)) {
                    items.push(r);
                }
            }
            _ => {}
        }
    }
    if lists.is_empty() {
        lists = Data::default().lists;
    }
    let first = lists[0].id;
    let list_of: HashMap<u64, u64> = items.iter().map(|r| (r.id, r.list)).collect();
    let top: HashSet<u64> = items.iter().filter(|r| r.parent.is_none()).map(|r| r.id).collect();
    for r in &mut items {
        if v1 || !list_ids.contains(&r.list) {
            r.list = first;
        }
        // Only one nesting level, within the same list.
        if let Some(p) = r.parent {
            let ok = top.contains(&p) && p != r.id && list_of.get(&p).is_some_and(|l| *l == r.list);
            if !ok {
                r.parent = None;
            }
        }
    }
    Data { lists, items }
}

pub fn load_checked(path: &Path) -> std::io::Result<Data> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Data::default()),
        Err(error) => return Err(error),
    };
    if meta.len() > MAX_BYTES as u64 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Reminders file too large",
        ));
    }
    let text = std::fs::read_to_string(path)?;
    if !matches!(text.lines().next(), Some(HEADER | HEADER_V1)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Unknown reminders format",
        ));
    }
    Ok(parse(&text))
}
pub fn load(path: &Path) -> Data {
    load_checked(path).unwrap_or_default()
}

fn invalid(msg: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, msg)
}

pub fn save(path: &Path, data: &Data) -> std::io::Result<()> {
    let mut list_ids = HashSet::new();
    let mut ids = HashSet::new();
    if data.items.len() > MAX_ITEMS
        || data.lists.is_empty()
        || data.lists.len() > MAX_LISTS
        || data.lists.iter().any(|l| !list_ids.insert(l.id) || unescape(&escaped(&l.name), MAX_NAME, false).is_none())
        || data.items.iter().any(|r| {
            !ids.insert(r.id)
                || !list_ids.contains(&r.list)
                || r.priority > 3
                || r.time.is_some_and(|t| t >= 1440)
                || unescape(&escaped(&r.title), MAX_TITLE, false).is_none()
                || unescape(&escaped(&r.notes), MAX_NOTES, true).is_none()
                || unescape(&escaped(&r.url), MAX_URL, true).is_none()
                || r.due.is_some_and(|d| !DAY_RANGE.contains(&d))
        })
    {
        return Err(invalid("Invalid reminder"));
    }
    // Validate serialized bytes before touching disk: multibyte text and
    // escaping can grow beyond the reader's limit despite valid item counts.
    let mut out = format!("{HEADER}\n");
    for l in &data.lists {
        out.push_str(&format!("L\t{}\t{}\t{}\t{}\n", l.id, l.color, l.icon, escaped(&l.name)));
    }
    let num = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    for r in &data.items {
        out.push_str(&format!(
            "R\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.id,
            r.list,
            num(r.parent.map(|p| p.to_string())),
            num(r.due.map(|d| d.to_string())),
            num(r.time.map(|t| t.to_string())),
            u8::from(r.completed),
            u8::from(r.flagged),
            r.priority,
            r.repeat.map_or_else(|| "-".into(), Repeat::encode),
            num(r.early.map(|e| e.to_string())),
            escaped(&r.title),
            escaped(&r.notes),
            escaped(&r.url),
        ));
        if out.len() > MAX_BYTES {
            return Err(invalid("Reminders file too large"));
        }
    }
    springboard::shell::fsutil::write_atomic(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repeat::Unit;
    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/test-reminders-store-{}-{name}/reminders.tsv",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        path
    }
    #[test]
    fn local_day_observes_offset() {
        assert_eq!(local_day(1_791_072_000 - 1800, 3600), local_day(1_791_072_000, 0));
        assert_eq!(local_day(-1, 0), -1);
    }
    #[test]
    fn v2_round_trips_every_field() {
        let path = scratch("roundtrip");
        let mut data = Data::default();
        data.lists.push(ListDef { id: 2, name: "Work\tTab".into(), color: 3, icon: 2 });
        let mut a = Reminder::new(1, "Call \"Dad\"\\home\n世界");
        a.notes = "line1\nline2".into();
        a.url = "https://example.com/a?b=1".into();
        a.list = 2;
        a.due = Some(20_000);
        a.time = Some(615);
        a.flagged = true;
        a.priority = 2;
        a.repeat = Some(Repeat::new(Unit::Week, 2));
        a.early = Some(60);
        let mut b = Reminder::new(2, "Sub");
        b.list = 2;
        b.parent = Some(1);
        b.completed = true;
        data.items = vec![a, b];
        save(&path, &data).unwrap();
        assert_eq!(load_checked(&path).unwrap(), data);
        assert!(!path.with_extension("tsv.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn v1_files_still_load_into_the_default_list() {
        let data = parse(&format!("{HEADER_V1}\n1\t-\t0\tCall \"Dad\"\\t\\\\home\\n世界\n1\t-\t1\tDuplicate\n2\tBAD\t0\tBad\n3\t-\t2\tBad\n4\t-\t0\tBad\\q\n5\t-\t0\t   \n6\t999999999\t0\tBad\n7\t100\t1\tGood\n"));
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].title, "Call \"Dad\"\t\\home\n世界");
        assert_eq!((data.items[1].id, data.items[1].due, data.items[1].completed), (7, Some(100), true));
        assert_eq!(data.lists, Data::default().lists);
        assert!(data.items.iter().all(|r| r.list == data.lists[0].id));
    }
    #[test]
    fn malformed_v2_rows_are_skipped_and_orphans_repaired() {
        let good = "R\t1\t1\t-\t-\t-\t0\t0\t0\t-\t-\tTop\t\t";
        let text = format!(
            "{HEADER}\nL\t1\t0\t0\tReminders\nL\t1\t0\t0\tDup\nL\tx\t0\t0\tBad\n{good}\n\
             R\t2\t1\t1\t-\t-\t0\t0\t0\t-\t-\tKid\t\t\n\
             R\t3\t1\t2\t-\t-\t0\t0\t0\t-\t-\tGrandkid\t\t\n\
             R\t4\t9\t-\t-\t-\t0\t0\t0\t-\t-\tLost list\t\t\n\
             R\t5\t1\t-\t-\t1500\t0\t0\t0\t-\t-\tBad time\t\t\n\
             R\t6\t1\t-\t-\t-\t0\t0\t7\t-\t-\tBad prio\t\t\n\
             R\t7\t1\t-\t-\t-\t0\t0\t0\tq9\t-\tBad repeat\t\t\n\
             R\t8\t1\t-\t-\t-\t0\t0\t0\t-\t-\t \t\t\n\
             garbage\n"
        );
        let data = parse(&text);
        assert_eq!(data.lists.len(), 1);
        let ids: Vec<u64> = data.items.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        assert_eq!(data.items[1].parent, Some(1));
        assert_eq!(data.items[2].parent, None, "second nesting level is flattened");
        assert_eq!(data.items[3].list, 1, "unknown list falls back to the first");
        assert!(parse("future-format\nR\t1").items.is_empty());
    }
    #[test]
    fn save_rejects_invalid_data() {
        let path = scratch("invalid");
        let mut data = Data::default();
        data.items = vec![Reminder::new(1, "ok"), Reminder::new(1, "dup")];
        assert!(save(&path, &data).is_err());
        data.items = vec![Reminder::new(1, "  ")];
        assert!(save(&path, &data).is_err());
        let mut x = Reminder::new(1, "x");
        x.list = 42;
        data.items = vec![x];
        assert!(save(&path, &data).is_err());
        data.items = vec![];
        data.lists.clear();
        assert!(save(&path, &data).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn unicode_byte_limit_rejects_oversized_and_leaves_no_temp_file() {
        let path = scratch("bytes");
        let mut data = Data::default();
        let mut id = 0;
        let mut size = 0;
        while size < MAX_BYTES + 10_000 {
            id += 1;
            let mut r = Reminder::new(id, "🧠".repeat(MAX_TITLE));
            r.notes = "世".repeat(MAX_NOTES);
            size += r.title.len() + r.notes.len();
            data.items.push(r);
            if data.items.len() >= MAX_ITEMS {
                break;
            }
        }
        assert_eq!(save(&path, &data).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        assert!(!path.exists());
        assert!(!path.with_extension("tsv.tmp").exists());
        data.items.truncate(10);
        save(&path, &data).unwrap();
        assert_eq!(load_checked(&path).unwrap().items.len(), 10);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
