//! Golden tests for the locale-aware formatters (en_US, is_IS, de_DE) and the calendar/zone pairing.
use bifrost_app::calendar_system::{days_from_civil, Calendar, CalendarIdentifier, DateComponents};
use bifrost_app::formatters::date_components::{self, UnitsStyle};
use bifrost_app::formatters::number::{self, Style};
use bifrost_app::formatters::relative::named_days;
use bifrost_app::formatters::*;
use bifrost_app::time_zone::TimeZone;

fn en() -> Locale {
    Locale::new("en_US")
}
fn is() -> Locale {
    Locale::new("is_IS")
}
fn de() -> Locale {
    Locale::new("de_DE")
}

#[test]
fn decimal_grouping_follows_the_locale() {
    let n = |l: Locale| NumberFormatter::new(l, NumberStyle::Decimal).string(1_234_567.891);
    assert_eq!(n(en()), "1,234,567.891");
    assert_eq!(n(is()), "1.234.567,891");
    assert_eq!(n(de()), "1.234.567,891");
}

#[test]
fn percent_and_currency_goldens() {
    let pct = |l: Locale| NumberFormatter::new(l, NumberStyle::Percent).string(0.256);
    assert_eq!(pct(en()), "26%");
    assert_eq!(pct(de()), "26\u{a0}%");
    let money = |l: Locale, code: &str, v: f64| {
        let mut f = NumberFormatter::new(l, NumberStyle::Currency);
        f.currency_code = Some(code.into());
        f.string(v)
    };
    assert_eq!(money(en(), "USD", 1234.5), "$1,234.50");
    assert_eq!(money(de(), "EUR", 1234.5), "1.234,50\u{a0}\u{20ac}");
    assert_eq!(money(is(), "ISK", 1234.0), "1.234\u{a0}kr.");
}

#[test]
fn spell_out_and_ordinals() {
    let s = |v| number::spell_out(&en(), v);
    assert_eq!(s(0.0), "zero");
    assert_eq!(s(21.0), "twenty-one");
    assert_eq!(s(100.0), "one hundred");
    assert_eq!(s(1234.0), "one thousand two hundred thirty-four");
    assert_eq!(s(2_000_001.0), "two million one");
    assert_eq!(s(-7.0), "minus seven");
    assert_eq!(s(1.5), "one point five");
    assert_eq!(number::format(&en(), Style::SpellOut, 15.0), "fifteen");
    let o = |n| number::ordinal(&en(), n);
    assert_eq!((o(1).as_str(), o(2).as_str(), o(3).as_str(), o(4).as_str()), ("1st", "2nd", "3rd", "4th"));
    assert_eq!((o(11).as_str(), o(12).as_str(), o(13).as_str(), o(21).as_str(), o(112).as_str()), ("11th", "12th", "13th", "21st", "112th"));
    assert_eq!(number::ordinal(&is(), 3), "3.");
    assert_eq!(number::ordinal(&de(), 3), "3.");
    assert_eq!(number::format(&en(), Style::Ordinal, 22.0), "22nd");
}

#[test]
fn date_patterns_use_locale_names() {
    let t = days_from_civil(2026, 10, 13) * 86_400;
    let mut f = DateFormatter::new(en());
    f.set_date_format("EEEE, MMMM d, y");
    assert_eq!(f.string(t), "Tuesday, October 13, 2026");
    let mut f = DateFormatter::new(is());
    f.set_date_format("d. MMMM y");
    assert_eq!(f.string(t), "13. október 2026");
    let mut f = DateFormatter::new(en());
    f.set_date_format("yyyy-MM-dd HH:mm");
    assert_eq!(f.string(0), "1970-01-01 00:00");
}

#[test]
fn relative_goldens() {
    assert_eq!(relative_date_time(&en(), 3 * 86_400), "in 3 days");
    assert_eq!(relative_date_time(&en(), -2 * 3600), "2 hours ago");
    assert_eq!(named_days(&en(), -1), "yesterday");
    assert_eq!(named_days(&en(), 0), "today");
    assert_eq!(named_days(&en(), 1), "tomorrow");
    assert_eq!(named_days(&en(), 3), "in 3 days");
    assert_eq!(named_days(&is(), -1), "í gær");
}

#[test]
fn byte_count_and_list_goldens() {
    assert_eq!(byte_count(&en(), 999), "999 bytes");
    assert_eq!(byte_count(&en(), 1_500_000), "1.5 MB");
    assert_eq!(byte_count(&is(), 1_500_000), "1,5 MB");
    assert_eq!(list(&en(), &["a", "b", "c"]), "a, b, and c");
    assert_eq!(list(&en(), &["a", "b"]), "a and b");
    assert_eq!(list(&is(), &["a", "b", "c"]), "a, b og c");
}

#[test]
fn measurement_converts_and_formats() {
    let m = Measurement::new(1.0, Unit::Kilometers).converted(Unit::Meters).unwrap();
    assert_eq!(m.value, 1000.0);
    assert!(Measurement::new(1.0, Unit::Meters).converted(Unit::Grams).is_none());
}

#[test]
fn duration_goldens() {
    let d = |s, st| date_components::string(s, st);
    assert_eq!(d(3723, UnitsStyle::Positional), "1:02:03");
    assert_eq!(d(307, UnitsStyle::Positional), "5:07");
    assert_eq!(d(3723, UnitsStyle::Abbreviated), "1h 2m 3s");
    assert_eq!(d(3723, UnitsStyle::Short), "1 hr, 2 min, 3 sec");
    assert_eq!(d(3723, UnitsStyle::Full), "1 hour, 2 minutes, 3 seconds");
    assert_eq!(d(0, UnitsStyle::Full), "0 seconds");
    assert_eq!(d(90_000, UnitsStyle::Full), "1 day, 1 hour");
}

#[test]
fn a_calendar_in_a_zone_formats_local_dates() {
    let ny = TimeZone::from_posix_rule("America/New_York", "EST5EDT,M3.2.0,M11.1.0").unwrap();
    let cal = Calendar::new(CalendarIdentifier::Gregorian, ny);
    let instant = cal.date(&DateComponents::ymd(2026, 7, 4).hms(9, 30, 0)).unwrap();
    let mut f = DateFormatter::new(en());
    f.tz_offset = i64::from(cal.time_zone.seconds_from_gmt(bifrost_app::coding::Date(instant as f64)));
    f.set_date_format("MMM d, h:mm a");
    assert_eq!(f.string(instant), "Jul 4, 9:30 AM");
}
