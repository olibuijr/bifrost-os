//! Card thumbnails: a contact (vCard), a calendar event (iCalendar) and the icon-with-badge answer for archives and
//! packages.

use super::canvas::{Canvas, ACCENT, INK, MUTED, RED};
use super::text::{read_head, text_from_bytes, HEAD_BYTES};
use bifrost_render::gfx::pack;
use bifrost_render::ui::{Weight, WHITE};
use bifrost_ui::thumb_service::{badge_for, SourceFile, ThumbArt, ThumbProvider};
use bifrost_ui::vcard;

const CARD_W: f32 = 200.0;
const CARD_H: f32 = 260.0;

fn canvas_for(px: u32) -> Option<Canvas> {
    let h = px as usize;
    let w = ((px as f32 * CARD_W / CARD_H).round() as usize).max(1);
    let mut c = Canvas::new(w, h, CARD_W)?;
    c.page(6.0);
    Some(c)
}

/// The letters in the avatar: the first letters of the given and family names, else of the first two words.
pub fn initials(card: &vcard::Card) -> String {
    let pick = |s: &str| s.trim().chars().next().filter(|c| c.is_alphanumeric());
    let mut out: String = [pick(&card.given_name), pick(&card.family_name)].into_iter().flatten().collect();
    if out.is_empty() {
        out = card.display_name().split_whitespace().filter_map(pick).take(2).collect();
    }
    out.to_uppercase()
}

/// An avatar colour that depends on the name only, so a contact keeps its colour.
pub fn avatar_color(name: &str) -> u32 {
    const PALETTE: [(u8, u8, u8); 6] = [(255, 149, 0), (52, 199, 89), (90, 200, 250), (175, 82, 222), (255, 45, 85), (88, 86, 214)];
    let h = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = PALETTE[h as usize % PALETTE.len()];
    pack(r, g, b, 255)
}

/// Contacts: the avatar, the name, the organization and the first phone number and email.
pub struct ContactProvider;

impl ThumbProvider for ContactProvider {
    fn name(&self) -> &'static str {
        "contact"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.vcard"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let text = text_from_bytes(&read_head(&file.path, HEAD_BYTES * 4)?)?;
        // a card cut off by the head limit still reads: close it
        let cards = vcard::parse(&text).or_else(|_| vcard::parse(&format!("{text}\nEND:VCARD\n"))).ok()?;
        contact_card(cards.first()?, px)
    }
}

pub fn contact_card(card: &vcard::Card, px: u32) -> Option<ThumbArt> {
    let name = card.display_name();
    if name.is_empty() && card.phones.is_empty() && card.emails.is_empty() {
        return None;
    }
    let mut c = canvas_for(px)?;
    let r = 36.0;
    c.rrect(CARD_W / 2.0 - r, 36.0, 2.0 * r, 2.0 * r, r, avatar_color(&name));
    c.text_centered(&initials(card), 26.0, Weight::Semibold, WHITE, CARD_W / 2.0, 36.0 + r - 15.0, 2.0 * r);
    let mut y = 124.0;
    c.text_centered(&name, 14.0, Weight::Bold, INK, CARD_W / 2.0, y, CARD_W - 24.0);
    y += 22.0;
    if !card.organization.trim().is_empty() {
        c.text_centered(card.organization.trim(), 9.5, Weight::Regular, MUTED, CARD_W / 2.0, y, CARD_W - 24.0);
        y += 22.0;
    }
    for v in card.phones.iter().take(1).chain(card.emails.iter().take(1)) {
        c.text_centered(&v.value, 9.0, Weight::Regular, ACCENT, CARD_W / 2.0, y, CARD_W - 24.0);
        y += 16.0;
    }
    c.into_pixels().map(ThumbArt::Pixels)
}

/// What the thumbnail shows of an event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub summary: String,
    pub location: String,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// Hour and minute when the event has a time of day.
    pub time: Option<(u32, u32)>,
}

/// The first event of an iCalendar text: continuation lines are unfolded and the properties `SUMMARY`, `LOCATION` and
/// `DTSTART` read. `None` when there is no event or its start is not a date.
pub fn first_event(text: &str) -> Option<Event> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").split('\n') {
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    let start = lines.iter().position(|l| l.eq_ignore_ascii_case("BEGIN:VEVENT"))?;
    let mut ev = Event::default();
    let mut dated = false;
    for l in &lines[start + 1..] {
        if l.eq_ignore_ascii_case("END:VEVENT") {
            break;
        }
        let Some((key, value)) = l.split_once(':') else { continue };
        // parameters (`DTSTART;VALUE=DATE`) follow the property name
        let name = key.split(';').next().unwrap_or("").to_ascii_uppercase();
        match name.as_str() {
            "SUMMARY" => ev.summary = unescape(value),
            "LOCATION" => ev.location = unescape(value),
            "DTSTART" => {
                let b = value.as_bytes();
                let num = |r: std::ops::Range<usize>| value.get(r).and_then(|s| s.parse::<u32>().ok());
                if b.len() >= 8 {
                    if let (Some(y), Some(m), Some(d)) = (num(0..4), num(4..6), num(6..8)) {
                        if (1..=12).contains(&m) && (1..=31).contains(&d) {
                            ev.year = y as i32;
                            ev.month = m;
                            ev.day = d;
                            ev.time = (b.get(8) == Some(&b'T')).then(|| num(9..11).zip(num(11..13))).flatten();
                            dated = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    dated.then_some(ev)
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/// Calendar events: the month over the day number, then the title, the time and the place.
pub struct CalendarProvider;

impl ThumbProvider for CalendarProvider {
    fn name(&self) -> &'static str {
        "calendar"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.calendar-event"]
    }
    fn render(&self, file: &SourceFile, px: u32) -> Option<ThumbArt> {
        let text = text_from_bytes(&read_head(&file.path, HEAD_BYTES * 4)?)?;
        event_card(&first_event(&text)?, px)
    }
}

pub fn event_card(ev: &Event, px: u32) -> Option<ThumbArt> {
    let mut c = canvas_for(px)?;
    c.rect(0.0, 0.0, CARD_W, 44.0, RED);
    c.text_centered(MONTHS[(ev.month as usize).clamp(1, 12) - 1], 18.0, Weight::Bold, WHITE, CARD_W / 2.0, 12.0, CARD_W);
    c.text_centered(&ev.day.to_string(), 64.0, Weight::Light, INK, CARD_W / 2.0, 54.0, CARD_W);
    let mut y = 150.0;
    if !ev.summary.is_empty() {
        c.text_centered(&ev.summary, 12.0, Weight::Bold, INK, CARD_W / 2.0, y, CARD_W - 24.0);
        y += 20.0;
    }
    if let Some((h, m)) = ev.time {
        c.text_centered(&format!("{h:02}:{m:02}"), 10.0, Weight::Regular, MUTED, CARD_W / 2.0, y, CARD_W - 24.0);
        y += 16.0;
    }
    if !ev.location.is_empty() {
        c.text_centered(&ev.location, 9.5, Weight::Regular, MUTED, CARD_W / 2.0, y, CARD_W - 24.0);
    }
    c.into_pixels().map(ThumbArt::Pixels)
}

/// The badge of an archive or package: the compound extensions read as one word ("TGZ"), the rest are the extension.
pub fn package_badge(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for (suffix, badge) in [(".pkg.tar.zst", "PKG"), (".tar.gz", "TGZ"), (".tar.xz", "TXZ"), (".tar.bz2", "TBZ"), (".tar.zst", "TZST")] {
        if lower.ends_with(suffix) {
            return Some(badge.to_string());
        }
    }
    badge_for(name)
}

/// Archives and installable packages: the type icon with a badge naming the format. The contents are not opened.
pub struct PackageProvider;

impl ThumbProvider for PackageProvider {
    fn name(&self) -> &'static str {
        "package"
    }
    fn types(&self) -> &'static [&'static str] {
        &["public.archive"]
    }
    fn render(&self, file: &SourceFile, _px: u32) -> Option<ThumbArt> {
        Some(ThumbArt::Icon { type_id: file.type_id.clone(), badge: package_badge(&file.name()) })
    }
}

#[cfg(test)]
mod tests {
    use super::super::canvas::have_font;
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn initials_come_from_the_names() {
        let c = vcard::parse("BEGIN:VCARD\nVERSION:3.0\nN:Smith;ada;;;\nFN:Ada Smith\nEND:VCARD\n").unwrap();
        assert_eq!(initials(&c[0]), "AS");
        let c = vcard::parse("BEGIN:VCARD\nVERSION:3.0\nFN:Grace Brewster Hopper\nEND:VCARD\n").unwrap();
        assert_eq!(initials(&c[0]), "GB");
    }

    #[test]
    fn a_name_keeps_its_colour() {
        assert_eq!(avatar_color("Ada"), avatar_color("Ada"));
    }

    #[test]
    fn an_event_is_read_through_folded_lines_and_parameters() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Lunch\\, with\r\n  Ada\r\nDTSTART;TZID=X:20261013T093000\r\nLOCATION:Cafe\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let ev = first_event(ics).unwrap();
        assert_eq!(ev.summary, "Lunch, with Ada");
        assert_eq!((ev.year, ev.month, ev.day, ev.time), (2026, 10, 13, Some((9, 30))));
        assert_eq!(ev.location, "Cafe");
    }

    #[test]
    fn an_all_day_event_has_no_time() {
        let ev = first_event("BEGIN:VEVENT\nDTSTART;VALUE=DATE:20270101\nSUMMARY:New year\nEND:VEVENT\n").unwrap();
        assert_eq!((ev.month, ev.day, ev.time), (1, 1, None));
    }

    #[test]
    fn text_without_a_dated_event_has_none() {
        assert!(first_event("BEGIN:VCALENDAR\nEND:VCALENDAR\n").is_none());
        assert!(first_event("BEGIN:VEVENT\nSUMMARY:x\nEND:VEVENT\n").is_none());
        assert!(first_event("BEGIN:VEVENT\nDTSTART:20261399\nEND:VEVENT\n").is_none());
    }

    #[test]
    fn package_badges_name_the_format() {
        assert_eq!(package_badge("a.tar.gz").as_deref(), Some("TGZ"));
        assert_eq!(package_badge("foo-1.0-1-x86_64.pkg.tar.zst").as_deref(), Some("PKG"));
        assert_eq!(package_badge("x.deb").as_deref(), Some("DEB"));
        assert_eq!(package_badge("x.zip").as_deref(), Some("ZIP"));
        assert_eq!(package_badge("x"), None);
    }

    #[test]
    fn a_package_answers_with_an_icon_and_a_badge() {
        let f = SourceFile::new(PathBuf::from("/d/x.rpm"), Default::default());
        assert_eq!(PackageProvider.render(&f, 64), Some(ThumbArt::Icon { type_id: "org.rpm.rpm-archive".into(), badge: Some("RPM".into()) }));
    }

    #[test]
    fn cards_are_drawn() {
        if !have_font() {
            return;
        }
        let c = vcard::parse("BEGIN:VCARD\nVERSION:3.0\nFN:Ada Smith\nORG:Analytical\nTEL:555\nEND:VCARD\n").unwrap();
        let Some(ThumbArt::Pixels(p)) = contact_card(&c[0], 128) else { panic!("pixels") };
        assert_eq!((p.w, p.h), (98, 128));
        let ev = first_event("BEGIN:VEVENT\nDTSTART:20261013T090000\nSUMMARY:Standup\nEND:VEVENT\n").unwrap();
        let Some(ThumbArt::Pixels(p)) = event_card(&ev, 128) else { panic!("pixels") };
        assert_eq!(p.px[3 * p.w + 5], RED);
    }
}
