//! Bounded, escaped, atomically written store. Format v3 keeps lists, list groups, sections, templates, smart lists and
//! full reminders; the v2 format (before groups and sections) and the v1 date-only format still load. A line starts with its kind: `L` list,
//! `G` group, `S` section, `T` template, `U` template item, `M` smart list, `R` reminder. Older files have shorter `L` and `R`
//! lines; the missing fields read as unset.
use crate::model::{
    Data, DueRule, FlagRule, ListDef, ListGroup, Reminder, Section, SmartDef, SmartRules, Template, TemplateItem, TimeRule, MAX_GROUPS,
    MAX_ITEMS, MAX_LISTS, MAX_NAME, MAX_NOTES, MAX_SECTIONS, MAX_SMARTS, MAX_TEMPLATES, MAX_TEMPLATE_ITEMS, MAX_TITLE, MAX_URL,
};
use crate::attach;
use crate::repeat::Repeat;
use crate::zones;
use springboard::shell::prefs::{escape_field as escaped, unescape_field};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Lines read after the header: reminders, lists, groups, sections, templates and their items.
const MAX_LINES: usize = 25_000;
/// The format this version writes. An older version does not know it and leaves the file alone (it reports that it
/// cannot read it and saves nothing), instead of reading the longer lines as damaged and dropping them.
const HEADER: &str = "springboard-reminders-v4";
/// The format of versions 1.14 to 1.17: reminders end at their pictures, and their places are not kept.
const HEADER_V3: &str = "springboard-reminders-v3";
/// The format of versions before 1.14: lists and reminders with fewer fields on their lines.
const HEADER_V2: &str = "springboard-reminders-v2";
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
    // R id list parent due time done flag prio repeat early title notes url [created [section [zone [urgent [pictures [place]]]]]]
    if !(14..=20).contains(&fields.len()) {
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
        created: match fields.get(14) {
            Some(f) => opt_num(f).ok()?,
            None => None,
        },
        section: match fields.get(15) {
            Some(f) => opt_num(f).ok()?,
            None => None,
        },
        tz: match fields.get(16) {
            None | Some(&"-") => None,
            Some(f) => Some(unescape(f, zones::MAX_NAME, false).filter(|n| zones::valid_name(n))?),
        },
        urgent: match fields.get(17) {
            Some(f) => flag(f)?,
            None => false,
        },
        attachments: match fields.get(18) {
            None | Some(&"-") => vec![],
            Some(f) => {
                let names: Vec<String> = f.split(',').map(str::to_string).collect();
                if names.len() > attach::MAX_PER_REMINDER || names.iter().any(|n| !attach::valid_name(n)) {
                    return None;
                }
                names
            }
        },
        place: match fields.get(19) {
            None | Some(&"-") => None,
            Some(f) => Some(crate::place::Place::decode(&unescape(f, crate::place::MAX_TEXT, false)?)?),
        },
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

fn parse_list(fields: &[&str]) -> Option<(ListDef, Option<u64>)> {
    // L id color icon name [pinned group]
    if fields.len() != 5 && fields.len() != 7 {
        return None;
    }
    let mut list = ListDef::new(
        fields[1].parse().ok()?,
        unescape(fields[4], MAX_NAME, false)?,
        fields[2].parse().ok()?,
        fields[3].parse().ok()?,
    );
    let mut group = None;
    if fields.len() == 7 {
        list.pinned = flag(fields[5])?;
        group = opt_num(fields[6]).ok()?;
    }
    Some((list, group))
}

fn parse_template_item(fields: &[&str]) -> Option<(u64, TemplateItem)> {
    // U template sub flagged priority title notes url
    if fields.len() != 8 {
        return None;
    }
    Some((
        fields[1].parse().ok()?,
        TemplateItem {
            sub: flag(fields[2])?,
            flagged: flag(fields[3])?,
            priority: fields[4].parse().ok().filter(|p| *p <= 3)?,
            title: unescape(fields[5], MAX_TITLE, false)?,
            notes: unescape(fields[6], MAX_NOTES, true)?,
            url: unescape(fields[7], MAX_URL, true)?,
        },
    ))
}

fn parse_smart(fields: &[&str]) -> Option<SmartDef> {
    // M id color due time flag priority tag list name
    if fields.len() != 10 {
        return None;
    }
    Some(SmartDef {
        id: fields[1].parse().ok()?,
        color: fields[2].parse().ok()?,
        rules: SmartRules {
            due: DueRule::from_code(fields[3].parse().ok()?)?,
            time: TimeRule::from_code(fields[4].parse().ok()?)?,
            flagged: FlagRule::from_code(fields[5].parse().ok()?)?,
            priority: fields[6].parse().ok().filter(|p| *p <= 3)?,
            tag: opt_num(fields[7]).ok()?,
            list: opt_num(fields[8]).ok()?,
        },
        name: unescape(fields[9], MAX_NAME, false)?,
    })
}

pub fn parse(text: &str) -> Data {
    let mut lines = text.lines();
    let v1 = match lines.next() {
        Some(HEADER | HEADER_V3 | HEADER_V2) => false,
        Some(HEADER_V1) => true,
        _ => return Data::default(),
    };
    let mut lists: Vec<ListDef> = vec![];
    let mut items: Vec<Reminder> = vec![];
    let mut groups: Vec<ListGroup> = vec![];
    let mut templates: Vec<Template> = vec![];
    let mut smarts: Vec<SmartDef> = vec![];
    // (section id, list id, name) in file order; attached to their lists once every list is known
    let mut sections: Vec<(u64, u64, String)> = vec![];
    let mut list_ids = HashSet::new();
    let mut group_ids = HashSet::new();
    let mut ids = HashSet::new();
    let mut list_group: HashMap<u64, u64> = HashMap::new();
    for line in lines.take(MAX_LINES) {
        if v1 {
            if let Some(r) = parse_v1(line).filter(|r| ids.insert(r.id)) {
                items.push(r);
            }
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        match fields.first().copied() {
            Some("L") if lists.len() < MAX_LISTS => {
                if let Some((list, group)) = parse_list(&fields).filter(|(l, _)| list_ids.insert(l.id)) {
                    if let Some(g) = group {
                        list_group.insert(list.id, g);
                    }
                    lists.push(list);
                }
            }
            Some("G") if fields.len() == 3 && groups.len() < MAX_GROUPS => {
                let (Ok(id), Some(name)) = (fields[1].parse::<u64>(), unescape(fields[2], MAX_NAME, false)) else { continue };
                if group_ids.insert(id) {
                    groups.push(ListGroup { id, name });
                }
            }
            Some("S") if fields.len() == 4 => {
                let (Ok(id), Ok(list), Some(name)) = (fields[1].parse::<u64>(), fields[2].parse::<u64>(), unescape(fields[3], MAX_NAME, false)) else {
                    continue;
                };
                sections.push((id, list, name));
            }
            Some("T") if fields.len() == 5 && templates.len() < MAX_TEMPLATES => {
                let (Ok(id), Ok(color), Ok(icon), Some(name)) =
                    (fields[1].parse::<u64>(), fields[2].parse::<u8>(), fields[3].parse::<u8>(), unescape(fields[4], MAX_NAME, false))
                else {
                    continue;
                };
                if templates.iter().all(|t| t.id != id) {
                    templates.push(Template { id, name, color, icon, items: vec![] });
                }
            }
            Some("M") if smarts.len() < MAX_SMARTS => {
                if let Some(smart) = parse_smart(&fields).filter(|m| smarts.iter().all(|x| x.id != m.id)) {
                    smarts.push(smart);
                }
            }
            Some("U") => {
                if let Some((tid, item)) = parse_template_item(&fields) {
                    if let Some(t) = templates.iter_mut().find(|t| t.id == tid).filter(|t| t.items.len() < MAX_TEMPLATE_ITEMS) {
                        t.items.push(item);
                    }
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
    // Groups must exist; sections need their list, a unique id and room.
    for l in &mut lists {
        l.group = list_group.get(&l.id).copied().filter(|g| group_ids.contains(g));
    }
    // a smart list that names a list that is gone takes every list
    for m in &mut smarts {
        if m.rules.list.is_some_and(|l| !list_ids.contains(&l)) {
            m.rules.list = None;
        }
    }
    let mut section_ids = HashSet::new();
    for (id, list, name) in sections {
        if let Some(l) = lists.iter_mut().find(|l| l.id == list).filter(|l| l.sections.len() < MAX_SECTIONS) {
            if section_ids.insert(id) {
                l.sections.push(Section { id, name });
            }
        }
    }
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
        // A section belongs to the reminder's own list.
        if let Some(s) = r.section {
            let ok = lists.iter().any(|l| l.id == r.list && l.sections.iter().any(|x| x.id == s));
            if !ok {
                r.section = None;
            }
        }
    }
    Data { lists, items, groups, templates, smarts }
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
    if !matches!(text.lines().next(), Some(HEADER | HEADER_V2 | HEADER_V1)) {
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

/// Whether `data` is within every limit the store enforces and every reference in it resolves.
fn valid(data: &Data) -> bool {
    let name_ok = |name: &str| unescape(&escaped(name), MAX_NAME, false).is_some();
    let mut list_ids = HashSet::new();
    let mut ids = HashSet::new();
    let mut group_ids = HashSet::new();
    let mut section_ids = HashSet::new();
    let mut template_ids = HashSet::new();
    let mut smart_ids = HashSet::new();
    data.items.len() <= MAX_ITEMS
        && data.smarts.len() <= MAX_SMARTS
        && data.smarts.iter().all(|m| {
            smart_ids.insert(m.id) && name_ok(&m.name) && m.rules.priority <= 3 && m.rules.list.is_none_or(|l| data.lists.iter().any(|x| x.id == l))
        })
        && !data.lists.is_empty()
        && data.lists.len() <= MAX_LISTS
        && data.groups.len() <= MAX_GROUPS
        && data.templates.len() <= MAX_TEMPLATES
        && data.groups.iter().all(|g| group_ids.insert(g.id) && name_ok(&g.name))
        && data.lists.iter().all(|l| {
            list_ids.insert(l.id)
                && name_ok(&l.name)
                && l.group.is_none_or(|g| group_ids.contains(&g))
                && l.sections.len() <= MAX_SECTIONS
                && l.sections.iter().all(|s| section_ids.insert(s.id) && name_ok(&s.name))
        })
        && data.templates.iter().all(|t| {
            template_ids.insert(t.id)
                && name_ok(&t.name)
                && t.items.len() <= MAX_TEMPLATE_ITEMS
                && t.items.iter().all(|i| {
                    i.priority <= 3
                        && unescape(&escaped(&i.title), MAX_TITLE, false).is_some()
                        && unescape(&escaped(&i.notes), MAX_NOTES, true).is_some()
                        && unescape(&escaped(&i.url), MAX_URL, true).is_some()
                })
        })
        && data.items.iter().all(|r| {
            ids.insert(r.id)
                && list_ids.contains(&r.list)
                && r.priority <= 3
                && r.time.is_none_or(|t| t < 1440)
                && unescape(&escaped(&r.title), MAX_TITLE, false).is_some()
                && unescape(&escaped(&r.notes), MAX_NOTES, true).is_some()
                && unescape(&escaped(&r.url), MAX_URL, true).is_some()
                && r.tz.as_deref().is_none_or(zones::valid_name)
                && r.attachments.len() <= attach::MAX_PER_REMINDER
                && r.attachments.iter().all(|n| attach::valid_name(n))
                && r.place.as_ref().is_none_or(crate::place::Place::valid)
                && r.due.is_none_or(|d| DAY_RANGE.contains(&d))
                && r.section.is_none_or(|s| data.lists.iter().any(|l| l.id == r.list && l.sections.iter().any(|x| x.id == s)))
        })
}

pub fn save(path: &Path, data: &Data) -> std::io::Result<()> {
    if !valid(data) {
        return Err(invalid("Invalid reminder"));
    }
    // Validate serialized bytes before touching disk: multibyte text and
    // escaping can grow beyond the reader's limit despite valid item counts.
    let num = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    let mut out = format!("{HEADER}\n");
    for l in &data.lists {
        out.push_str(&format!(
            "L\t{}\t{}\t{}\t{}\t{}\t{}\n",
            l.id,
            l.color,
            l.icon,
            escaped(&l.name),
            u8::from(l.pinned),
            num(l.group.map(|g| g.to_string())),
        ));
    }
    for g in &data.groups {
        out.push_str(&format!("G\t{}\t{}\n", g.id, escaped(&g.name)));
    }
    for l in &data.lists {
        for s in &l.sections {
            out.push_str(&format!("S\t{}\t{}\t{}\n", s.id, l.id, escaped(&s.name)));
        }
    }
    for t in &data.templates {
        out.push_str(&format!("T\t{}\t{}\t{}\t{}\n", t.id, t.color, t.icon, escaped(&t.name)));
        for i in &t.items {
            out.push_str(&format!(
                "U\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                t.id,
                u8::from(i.sub),
                u8::from(i.flagged),
                i.priority,
                escaped(&i.title),
                escaped(&i.notes),
                escaped(&i.url),
            ));
        }
        if out.len() > MAX_BYTES {
            return Err(invalid("Reminders file too large"));
        }
    }
    for m in &data.smarts {
        out.push_str(&format!(
            "M\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            m.id,
            m.color,
            m.rules.due.code(),
            m.rules.time.code(),
            m.rules.flagged.code(),
            m.rules.priority,
            num(m.rules.tag.map(|k| k.to_string())),
            num(m.rules.list.map(|l| l.to_string())),
            escaped(&m.name),
        ));
    }
    for r in &data.items {
        out.push_str(&format!(
            "R\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
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
            num(r.created.map(|c| c.to_string())),
            num(r.section.map(|s| s.to_string())),
            r.tz.as_deref().map_or_else(|| "-".to_string(), escaped),
            u8::from(r.urgent),
            if r.attachments.is_empty() { "-".to_string() } else { r.attachments.join(",") },
            r.place.as_ref().map_or_else(|| "-".to_string(), |p| escaped(&p.encode())),
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
    fn the_header_written_is_the_one_an_older_version_refuses() {
        let path = scratch("header");
        save(&path, &Data::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().next(), Some("springboard-reminders-v4"));
        assert!(load_checked(&path).is_ok());
        // files of the previous formats load, and a future one is refused whole
        std::fs::write(&path, "springboard-reminders-v2\nL\t1\t0\t0\tOne\n").unwrap();
        assert_eq!(load_checked(&path).unwrap().lists[0].name, "One");
        std::fs::write(&path, "springboard-reminders-v3\nL\t1\t0\t0\tOne\n").unwrap();
        assert_eq!(load_checked(&path).unwrap().lists[0].name, "One");
        std::fs::write(&path, "springboard-reminders-v5\nL\t1\t0\t0\tOne\n").unwrap();
        assert!(load_checked(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
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
        let mut work = ListDef::new(2, "Work\tTab", 3, 2);
        work.pinned = true;
        work.group = Some(4);
        work.sections = vec![Section { id: 9, name: "Cold\nfood".into() }, Section { id: 12, name: "Dry".into() }];
        data.lists.push(work);
        data.groups.push(ListGroup { id: 4, name: "Home\tlife".into() });
        data.smarts.push(SmartDef {
            id: 6,
            name: "Morning\nflags".into(),
            color: 2,
            rules: SmartRules { due: DueRule::Next7, time: TimeRule::Morning, flagged: FlagRule::Flagged, priority: 2, tag: Some(u64::MAX), list: Some(2) },
        });
        data.templates.push(Template {
            id: 3,
            name: "Trip".into(),
            color: 2,
            icon: 1,
            items: vec![
                TemplateItem { title: "Pack \"bags\"".into(), notes: "line1\nline2".into(), url: "https://example.com".into(), flagged: true, priority: 3, sub: false },
                TemplateItem { title: "Passports".into(), notes: String::new(), url: String::new(), flagged: false, priority: 0, sub: true },
            ],
        });
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
        a.created = Some(1_791_000_000);
        a.section = Some(9);
        a.tz = Some("America/New_York".into());
        a.urgent = true;
        a.attachments = vec!["att-1791073380-1.png".into(), "att-1791073380-2.jpg".into()];
        let mut b = Reminder::new(2, "Sub");
        b.list = 2;
        b.parent = Some(1);
        b.completed = true;
        let mut c = Reminder::new(3, "Pick up the parcel");
        c.list = 2;
        c.place = Some(crate::place::Place { name: "Post office, Hverfisgata".into(), lat: 64.1505, lon: -21.9330, radius_m: 150, trigger: crate::place::Trigger::Arriving });
        data.items = vec![a, b, c];
        save(&path, &data).unwrap();
        assert_eq!(load_checked(&path).unwrap(), data);
        assert!(!path.with_extension("tsv.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn files_from_before_groups_sections_and_creation_dates_still_load() {
        let text = format!(
            "{HEADER_V2}\nL\t1\t0\t0\tReminders\nL\t2\t1\t1\tWork\nR\t1\t1\t-\t-\t-\t0\t0\t0\t-\t-\tOld\t\t\nR\t2\t1\t-\t100\t-\t1\t0\t0\t-\t-\tOlder\t\t\t500\n"
        );
        let data = parse(&text);
        assert_eq!(data.lists.len(), 2);
        assert!(data.lists.iter().all(|l| !l.pinned && l.group.is_none() && l.sections.is_empty()));
        assert_eq!((data.items[0].created, data.items[0].section), (None, None));
        assert_eq!((data.items[1].created, data.items[1].section), (Some(500), None));
        assert!(data.groups.is_empty() && data.templates.is_empty());
    }
    #[test]
    fn a_zone_that_is_not_zone_shaped_drops_the_reminder_and_the_flag_must_be_a_flag() {
        let ok = "R\t1\t1\t-\t-\t-\t0\t0\t0\t-\t-\tOk\t\t\t-\t-\tEurope/Paris\t1";
        let spaced = "R\t2\t1\t-\t-\t-\t0\t0\t0\t-\t-\tBad zone\t\t\t-\t-\tnot a zone\t0";
        let flag = "R\t3\t1\t-\t-\t-\t0\t0\t0\t-\t-\tBad flag\t\t\t-\t-\t-\t2";
        let device = "R\t4\t1\t-\t-\t-\t0\t0\t0\t-\t-\tDevice\t\t\t-\t-\t-\t0";
        let data = parse(&format!("{HEADER}\nL\t1\t0\t0\tOne\n{ok}\n{spaced}\n{flag}\n{device}\n"));
        let ids: Vec<u64> = data.items.iter().map(|r| r.id).collect();
        assert_eq!(ids, [1, 4]);
        assert_eq!((data.items[0].tz.as_deref(), data.items[0].urgent), (Some("Europe/Paris"), true));
        assert_eq!((data.items[1].tz.as_deref(), data.items[1].urgent), (None, false));
        let mut r = Reminder::new(1, "x");
        r.tz = Some("bad zone".into());
        let mut d = Data::default();
        d.items = vec![r];
        assert!(save(&scratch("zone"), &d).is_err(), "save refuses a zone name that load would drop");
    }
    #[test]
    fn a_reminder_with_a_picture_name_that_is_not_ours_is_dropped() {
        let row = |id: u32, pictures: &str| format!("R\t{id}\t1\t-\t-\t-\t0\t0\t0\t-\t-\tT{id}\t\t\t-\t-\t-\t0\t{pictures}\n");
        let text = format!(
            "{HEADER}\nL\t1\t0\t0\tOne\n{}{}{}{}{}",
            row(1, "att-1-1.png,att-1-2.jpg"),
            row(2, "../../etc/passwd"),
            row(3, "att-1-1.png,att-1-2.png,att-1-3.png,att-1-4.png,att-1-5.png"),
            row(4, "-"),
            row(5, "att-1-1.gif"),
        );
        let data = parse(&text);
        let ids: Vec<u64> = data.items.iter().map(|r| r.id).collect();
        assert_eq!(ids, [1, 4]);
        assert_eq!(data.items[0].attachments, ["att-1-1.png", "att-1-2.jpg"]);
        assert!(data.items[1].attachments.is_empty());
        let mut r = Reminder::new(1, "x");
        r.attachments = vec!["../x.png".into()];
        let mut d = Data::default();
        d.items = vec![r];
        assert!(save(&scratch("pictures"), &d).is_err(), "save refuses a name that load would drop");
    }
    #[test]
    fn broken_references_in_the_new_fields_are_repaired() {
        let text = format!(
            "{HEADER}\nL\t1\t0\t0\tOne\t1\t7\nL\t2\t0\t0\tTwo\t0\t5\nG\t5\tHome\nG\t5\tDup\nG\tx\tBad\n\
             S\t10\t1\tA\nS\t10\t2\tDuplicate id\nS\t11\t99\tNo list\nS\t12\t2\tB\n\
             T\t1\t0\t0\tTrip\nT\t1\t0\t0\tDup\nU\t1\t0\t1\t3\tPack\t\t\nU\t1\t0\t0\t9\tBad priority\t\t\nU\t8\t0\t0\t0\tNo template\t\t\n\
             R\t1\t1\t-\t-\t-\t0\t0\t0\t-\t-\tIn A\t\t\t-\t10\n\
             R\t2\t1\t-\t-\t-\t0\t0\t0\t-\t-\tIn the other list's B\t\t\t-\t12\n\
             R\t3\t2\t-\t-\t-\t0\t0\t0\t-\t-\tBad created\t\t\tx\t-\n"
        );
        let data = parse(&text);
        assert!(data.lists[0].pinned);
        assert_eq!(data.lists[0].group, None, "group 7 does not exist");
        assert_eq!(data.lists[1].group, Some(5));
        assert_eq!(data.groups.len(), 1);
        assert_eq!(data.lists[0].sections.len(), 1, "a duplicate id and an unknown list are dropped");
        assert_eq!(data.lists[1].sections[0].name, "B");
        assert_eq!(data.templates.len(), 1);
        assert_eq!(data.templates[0].items.len(), 1, "a bad priority and an unknown template are dropped");
        assert_eq!(data.items.iter().map(|r| r.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(data.items[0].section, Some(10));
        assert_eq!(data.items[1].section, None, "a section of another list is dropped");
    }
    #[test]
    fn smart_lists_with_bad_codes_or_lists_are_repaired_or_dropped() {
        let text = format!(
            "{HEADER}\nL\t1\t0\t0\tOne\n\
             M\t1\t0\t2\t1\t1\t2\t-\t1\tGood\nM\t1\t0\t0\t0\t0\t0\t-\t-\tDuplicate id\n\
             M\t2\t0\t9\t0\t0\t0\t-\t-\tBad due code\nM\t3\t0\t0\t0\t0\t7\t-\t-\tBad priority\n\
             M\t4\t0\t0\t0\t0\t0\t-\t55\tGone list\nM\t5\t0\t0\t0\t0\t0\t-\t-\t \nM\t6\tx\t0\t0\t0\t0\t-\t-\tBad color\n"
        );
        let data = parse(&text);
        let names: Vec<&str> = data.smarts.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Good", "Gone list"]);
        assert_eq!(data.smarts[0].rules.due, DueRule::Today);
        assert_eq!(data.smarts[0].rules.list, Some(1));
        assert_eq!(data.smarts[1].rules.list, None, "a list that does not exist is dropped from the rule");
    }
    #[test]
    fn save_rejects_dangling_references_and_over_limit_collections() {
        let path = scratch("dangling");
        let mut data = Data::default();
        data.lists[0].group = Some(9);
        assert!(save(&path, &data).is_err(), "a group that does not exist");
        data.lists[0].group = None;
        let mut r = Reminder::new(1, "x");
        r.section = Some(3);
        data.items = vec![r];
        assert!(save(&path, &data).is_err(), "a section that does not exist");
        data.items.clear();
        data.lists[0].sections = (0..=MAX_SECTIONS as u64).map(|id| Section { id: id + 1, name: format!("S{id}") }).collect();
        assert!(save(&path, &data).is_err(), "too many sections");
        data.lists[0].sections.truncate(2);
        data.lists[0].sections[1].id = data.lists[0].sections[0].id;
        assert!(save(&path, &data).is_err(), "a duplicate section id");
        data.lists[0].sections.clear();
        data.templates = vec![Template { id: 1, name: "  ".into(), color: 0, icon: 0, items: vec![] }];
        assert!(save(&path, &data).is_err(), "a blank template name");
        data.templates.clear();
        data.smarts = vec![SmartDef { id: 1, name: "S".into(), color: 0, rules: SmartRules { list: Some(9), ..SmartRules::default() } }];
        assert!(save(&path, &data).is_err(), "a smart list that names a list that does not exist");
        assert!(!path.exists());
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
