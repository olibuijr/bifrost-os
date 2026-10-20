//! Language & Region (Settings): the preferred languages, the region whose formats the system and apps
//! use, the first day of the week and the currency. The shell keeps the choice in its `settings.ini`
//! (`languages`, `region`, `first_weekday`, `currency`), publishes it for its own drawing (`publish`,
//! `current`), and the SDK reads the same file, so every process formats dates and numbers alike.
//! Pure data and functions; the only state is the published copy.

use std::sync::RwLock;

/// The languages Settings offers: code and the language's own name.
pub const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("is", "Íslenska")];

/// The regions Settings offers: code and name.
pub const REGIONS: [(&str, &str); 12] = [
    ("IS", "Iceland"),
    ("US", "United States"),
    ("GB", "United Kingdom"),
    ("DK", "Denmark"),
    ("NO", "Norway"),
    ("SE", "Sweden"),
    ("FI", "Finland"),
    ("DE", "Germany"),
    ("FR", "France"),
    ("CA", "Canada"),
    ("AU", "Australia"),
    ("JP", "Japan"),
];

/// The region of a fresh install (the default time zone is Atlantic/Reykjavik).
pub const DEFAULT_REGION: &str = "IS";

/// The saved Language & Region choice. Empty or None fields follow the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    /// Preferred languages in order; the first one is the system's. Empty means English.
    pub languages: Vec<String>,
    /// ISO 3166-1 alpha-2 code; empty means `DEFAULT_REGION`.
    pub region: String,
    /// First day of the week, 0 = Sunday .. 6 = Saturday; None follows the region.
    pub first_weekday: Option<u32>,
    /// ISO 4217 code; None follows the region.
    pub currency: Option<String>,
}

impl Region {
    /// Reads the `languages`, `region`, `first_weekday` and `currency` keys from a settings lookup.
    /// Malformed values are ignored, so a damaged file gives the defaults.
    pub fn from_lookup<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Region {
        Region {
            languages: get("languages").map(parse_languages).unwrap_or_default(),
            region: get("region").and_then(clean_region).unwrap_or_default(),
            first_weekday: get("first_weekday").and_then(|v| v.trim().parse::<u32>().ok()).filter(|d| *d < 7),
            currency: get("currency").and_then(clean_currency),
        }
    }

    /// Reads the same keys from the text of a `settings.ini` (`key=value` lines, `#` comments).
    pub fn from_settings_text(text: &str) -> Region {
        let map: std::collections::HashMap<&str, &str> = text
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') {
                    return None;
                }
                l.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
            })
            .collect();
        Region::from_lookup(|k| map.get(k).copied())
    }

    /// The language the system uses ("en", "is").
    pub fn language(&self) -> &str {
        self.languages.first().map_or("en", String::as_str)
    }

    /// The region in force ("IS" when none is chosen).
    pub fn region_code(&self) -> String {
        if self.region.is_empty() {
            DEFAULT_REGION.to_string()
        } else {
            self.region.clone()
        }
    }

    /// The locale identifier, e.g. "is_IS".
    pub fn identifier(&self) -> String {
        format!("{}_{}", self.language(), self.region_code())
    }

    /// The first day of the week, 0 = Sunday .. 6 = Saturday.
    pub fn first_weekday(&self) -> u32 {
        self.first_weekday.unwrap_or_else(|| default_first_weekday(&self.region_code()))
    }

    /// The currency code formats use.
    pub fn currency(&self) -> String {
        self.currency.clone().unwrap_or_else(|| default_currency(&self.region_code(), self.language()).to_string())
    }

    /// The region's name as Settings shows it ("Iceland"); the code itself when it is not in `REGIONS`.
    pub fn region_name(&self) -> String {
        let code = self.region_code();
        REGIONS.iter().find(|(c, _)| *c == code.as_str()).map_or(code.clone(), |(_, n)| n.to_string())
    }
}

/// The first day of the week of a region: Sunday in the United States, Canada and Japan, Monday elsewhere.
pub fn default_first_weekday(region: &str) -> u32 {
    match region {
        "US" | "CA" | "JP" => 0,
        _ => 1,
    }
}

/// The currency of a region, or of the language when the region is unknown (Icelandic: ISK, else USD).
pub fn default_currency(region: &str, language: &str) -> &'static str {
    match region {
        "IS" => "ISK",
        "GB" => "GBP",
        "DK" => "DKK",
        "NO" => "NOK",
        "SE" => "SEK",
        "CA" => "CAD",
        "AU" => "AUD",
        "JP" => "JPY",
        "US" => "USD",
        "DE" | "FR" | "FI" | "IE" | "AT" | "ES" | "IT" | "NL" => "EUR",
        _ => {
            if language == "is" {
                "ISK"
            } else {
                "USD"
            }
        }
    }
}

/// A `languages` value ("is,en") as codes: 2 or 3 lowercase letters each, no repeats, in order.
pub fn parse_languages(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(',') {
        let code = part.trim().to_ascii_lowercase();
        let ok = (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase());
        if ok && !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// A region code ("is" becomes "IS"); None unless it is two letters.
pub fn clean_region(text: &str) -> Option<String> {
    let code = text.trim().to_ascii_uppercase();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase())).then_some(code)
}

/// A currency code ("eur" becomes "EUR"); None unless it is three letters.
pub fn clean_currency(text: &str) -> Option<String> {
    let code = text.trim().to_ascii_uppercase();
    (code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())).then_some(code)
}

static PUBLISHED: RwLock<Option<Region>> = RwLock::new(None);

/// Makes `region` the choice this process draws with (the shell, after a start or a change).
pub fn publish(region: Region) {
    if let Ok(mut p) = PUBLISHED.write() {
        *p = Some(region);
    }
}

/// The published choice; the defaults until one is published.
pub fn current() -> Region {
    PUBLISHED.read().ok().and_then(|p| p.clone()).unwrap_or_default()
}

const DAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS_IS: [&str; 7] = ["sun.", "mán.", "þri.", "mið.", "fim.", "fös.", "lau."];
const MONTHS_IS: [&str; 12] = ["jan.", "feb.", "mar.", "apr.", "maí", "jún.", "júl.", "ágú.", "sep.", "okt.", "nóv.", "des."];

/// The short date of the menu bar and lock screen: "Tue 13 Oct" in most regions, "Tue Oct 13" in the
/// United States and "þri. 13. okt." in Icelandic. `wday` is 0 for Sunday, `mon` 0 for January.
pub fn short_date(region: &Region, wday: i32, mday: i32, mon: i32) -> String {
    let wd = wday.rem_euclid(7) as usize;
    let mo = mon.rem_euclid(12) as usize;
    if region.language() == "is" {
        format!("{} {mday}. {}", DAYS_IS[wd], MONTHS_IS[mo])
    } else if region.region_code() == "US" {
        format!("{} {} {mday}", DAYS_EN[wd], MONTHS_EN[mo])
    } else {
        format!("{} {mday} {}", DAYS_EN[wd], MONTHS_EN[mo])
    }
}

const DAYS_FULL_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS_FULL_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_FULL_IS: [&str; 7] = ["sunnudagur", "mánudagur", "þriðjudagur", "miðvikudagur", "fimmtudagur", "föstudagur", "laugardagur"];
const MONTHS_FULL_IS: [&str; 12] = ["janúar", "febrúar", "mars", "apríl", "maí", "júní", "júlí", "ágúst", "september", "október", "nóvember", "desember"];

/// The full weekday name in the region's language ("Tuesday", "þriðjudagur"). `wday` is 0 for Sunday.
pub fn day_name(region: &Region, wday: i32) -> &'static str {
    let wd = wday.rem_euclid(7) as usize;
    if region.language() == "is" { DAYS_FULL_IS[wd] } else { DAYS_FULL_EN[wd] }
}

/// The full month name in the region's language ("October", "október"). `mon` is 0 for January.
pub fn month_name(region: &Region, mon: i32) -> &'static str {
    let mo = mon.rem_euclid(12) as usize;
    if region.language() == "is" { MONTHS_FULL_IS[mo] } else { MONTHS_FULL_EN[mo] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_day_and_month_names_follow_the_language() {
        let en = Region::default();
        assert_eq!((day_name(&en, 2), month_name(&en, 9)), ("Tuesday", "October"));
        assert_eq!((day_name(&en, -5), month_name(&en, 12)), ("Tuesday", "January"), "out of range wraps");
        let is = Region::from_settings_text("languages=is\nregion=is\n");
        assert_eq!((day_name(&is, 2), month_name(&is, 9)), ("þriðjudagur", "október"));
    }

    #[test]
    fn a_fresh_install_is_english_in_iceland_with_monday_weeks_and_isk() {
        let r = Region::default();
        assert_eq!(r.language(), "en");
        assert_eq!(r.region_code(), "IS");
        assert_eq!(r.identifier(), "en_IS");
        assert_eq!(r.first_weekday(), 1);
        assert_eq!(r.currency(), "ISK");
        assert_eq!(r.region_name(), "Iceland");
    }

    #[test]
    fn the_settings_file_gives_the_choices_and_ignores_damage() {
        let r = Region::from_settings_text("# Language & Region\nlanguages=is, en\nregion=us\nfirst_weekday=0\ncurrency=eur\nclock_24h=1\n");
        assert_eq!(r.languages, vec!["is".to_string(), "en".to_string()]);
        assert_eq!(r.region, "US");
        assert_eq!(r.first_weekday, Some(0));
        assert_eq!(r.currency.as_deref(), Some("EUR"));
        assert_eq!(r.identifier(), "is_US");

        let bad = Region::from_settings_text("languages=xx1,,\nregion=ICE\nfirst_weekday=9\ncurrency=\n");
        assert_eq!(bad, Region::default());
    }

    #[test]
    fn a_region_follows_its_own_week_and_currency_unless_overridden() {
        let us = Region { region: "US".into(), ..Region::default() };
        assert_eq!((us.first_weekday(), us.currency().as_str()), (0, "USD"));
        let de = Region { region: "DE".into(), languages: vec!["en".into()], ..Region::default() };
        assert_eq!((de.first_weekday(), de.currency().as_str()), (1, "EUR"));
        let custom = Region { region: "DE".into(), first_weekday: Some(0), currency: Some("CHF".into()), ..Region::default() };
        assert_eq!((custom.first_weekday(), custom.currency().as_str()), (0, "CHF"));
    }

    #[test]
    fn region_names_fall_back_to_the_code() {
        assert_eq!(Region { region: "JP".into(), ..Region::default() }.region_name(), "Japan");
        assert_eq!(Region { region: "ZZ".into(), ..Region::default() }.region_name(), "ZZ");
    }

    #[test]
    fn the_short_date_follows_the_region_and_language() {
        assert_eq!(short_date(&Region::default(), 6, 3, 9), "Sat 3 Oct");
        assert_eq!(short_date(&Region::default(), 0, 31, 11), "Sun 31 Dec");
        let us = Region { region: "US".into(), ..Region::default() };
        assert_eq!(short_date(&us, 2, 13, 9), "Tue Oct 13");
        let is = Region { languages: vec!["is".into()], ..Region::default() };
        assert_eq!(short_date(&is, 2, 13, 9), "þri. 13. okt.");
    }
}
