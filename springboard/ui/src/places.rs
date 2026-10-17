//! City and country name tables: the places the system knows by name, with their zone, country and position.
//!
//! The tables come from the system time zone database (`iso3166.tab` for country codes and names, `zone1970.tab`
//! for one row per zone: countries, position, zone name), so nothing is bundled or fetched. The parsers take the
//! text, which keeps them testable; [`PlaceTable::load_system`] reads the installed files. A city is named by the
//! last part of its zone (`America/New_York` is "New York"). Country names can be overridden per language, and a
//! search folds case and the Icelandic letters so "reykjavik" and "Thor" find their places.

use std::collections::HashMap;

/// Where the installed tables live.
pub const SYSTEM_ISO3166: &str = "/usr/share/zoneinfo/iso3166.tab";
pub const SYSTEM_ZONES: &str = "/usr/share/zoneinfo/zone1970.tab";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    /// ISO 3166 alpha-2 code, upper case.
    pub code: String,
    /// English name from the table.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// City name derived from the zone: `Argentina/Buenos_Aires` is "Buenos Aires".
    pub city: String,
    /// The time zone name, for example `Atlantic/Reykjavik`.
    pub zone: String,
    /// Codes of the countries that use this zone (first is the main one).
    pub countries: Vec<String>,
    pub latitude: f64,
    pub longitude: f64,
}

/// A degree-minute(-second) position like `+4230+00131` or `-335242+1511222`, as (latitude, longitude) in degrees.
pub fn parse_coordinates(text: &str) -> Option<(f64, f64)> {
    let bytes = text.as_bytes();
    let split = bytes.iter().skip(1).position(|b| *b == b'+' || *b == b'-')? + 1;
    let (lat, lon) = text.split_at(split);
    Some((parse_part(lat, 2)?, parse_part(lon, 3)?))
}

fn parse_part(part: &str, degree_digits: usize) -> Option<f64> {
    let sign = match part.chars().next()? {
        '+' => 1.0,
        '-' => -1.0,
        _ => return None,
    };
    let digits = &part[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (deg, rest) = digits.split_at(degree_digits.min(digits.len()));
    let degrees: f64 = deg.parse().ok()?;
    let (minutes, seconds): (f64, f64) = match rest.len() {
        2 => (rest.parse().ok()?, 0.0),
        4 => (rest[..2].parse().ok()?, rest[2..].parse().ok()?),
        _ => return None,
    };
    if minutes >= 60.0 || seconds >= 60.0 {
        return None;
    }
    Some(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
}

/// Reads `iso3166.tab`: `CODE<TAB>Name` lines. Comment (`#`) and malformed lines are skipped.
pub fn parse_countries(text: &str) -> Vec<Country> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (code, name) = l.split_once('\t')?;
            let code = code.trim();
            if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) || name.trim().is_empty() {
                return None;
            }
            Some(Country { code: code.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

/// Reads `zone1970.tab` (or `zone.tab`): `countries<TAB>position<TAB>zone[<TAB>comment]`. Malformed lines are skipped.
pub fn parse_places(text: &str) -> Vec<Place> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let countries = cols.next()?;
            let (latitude, longitude) = parse_coordinates(cols.next()?)?;
            let zone = cols.next()?.trim();
            let city = zone.rsplit('/').next()?.replace('_', " ");
            if zone.is_empty() || city.is_empty() {
                return None;
            }
            let countries: Vec<String> = countries.split(',').map(|c| c.trim().to_string()).filter(|c| c.len() == 2).collect();
            if countries.is_empty() {
                return None;
            }
            Some(Place { city, zone: zone.to_string(), countries, latitude, longitude })
        })
        .collect()
}

/// Lower case with accents and the Icelandic letters reduced to plain ones, so typed text matches either spelling.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(|c| c.to_lowercase()) {
        match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'ð' => out.push('d'),
            'þ' => out.push_str("th"),
            'æ' => out.push_str("ae"),
            '_' | '-' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct PlaceTable {
    countries: Vec<Country>,
    places: Vec<Place>,
    /// (language, code) -> name in that language.
    localized: HashMap<(String, String), String>,
}

impl PlaceTable {
    pub fn from_texts(iso3166: &str, zones: &str) -> PlaceTable {
        PlaceTable { countries: parse_countries(iso3166), places: parse_places(zones), localized: HashMap::new() }
    }

    /// The installed tables, or an empty table when the files are missing.
    pub fn load_system() -> PlaceTable {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        PlaceTable::from_texts(&read(SYSTEM_ISO3166), &read(SYSTEM_ZONES))
    }

    pub fn countries(&self) -> &[Country] {
        &self.countries
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    /// Sets the name of a country in a language (`"is"`, `"IS"`, "Ísland").
    pub fn set_country_name(&mut self, language: &str, code: &str, name: &str) {
        self.localized.insert((language.to_ascii_lowercase(), code.to_ascii_uppercase()), name.to_string());
    }

    /// The name of a country in `language`, falling back to the English table name.
    pub fn country_name(&self, code: &str, language: &str) -> Option<&str> {
        let code = code.to_ascii_uppercase();
        if let Some(n) = self.localized.get(&(language.to_ascii_lowercase(), code.clone())) {
            return Some(n);
        }
        self.countries.iter().find(|c| c.code == code).map(|c| c.name.as_str())
    }

    /// The place of a zone name.
    pub fn place_for_zone(&self, zone: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.zone == zone)
    }

    /// Places whose city, zone or country name starts a word with the query, best first: a city that starts with the
    /// query comes before one that has a word that does, which comes before a country match. At most `limit` places.
    pub fn search(&self, query: &str, language: &str, limit: usize) -> Vec<&Place> {
        let q = fold(query.trim());
        if q.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u8, &Place)> = self
            .places
            .iter()
            .filter_map(|p| {
                let city = fold(&p.city);
                let rank = if city.starts_with(&q) {
                    0
                } else if city.split(' ').any(|w| w.starts_with(&q)) {
                    1
                } else if p.countries.iter().any(|c| self.country_name(c, language).map_or(false, |n| fold(n).split(' ').any(|w| w.starts_with(&q)))) {
                    2
                } else {
                    return None;
                };
                Some((rank, p))
            })
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.city.cmp(&b.1.city)));
        scored.into_iter().take(limit).map(|(_, p)| p).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISO: &str = "# header\nAD\tAndorra\nIS\tIceland\nUS\tUnited States\nXX\n\nzz\tlower\n";
    const ZONES: &str = "# header\nAD\t+4230+00131\tEurope/Andorra\nIS\t+6409-02151\tAtlantic/Reykjavik\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\nUS\tbad\tAmerica/Nowhere\n";

    fn table() -> PlaceTable {
        PlaceTable::from_texts(ISO, ZONES)
    }

    #[test]
    fn countries_skip_comments_and_malformed_lines() {
        let t = table();
        assert_eq!(t.countries().len(), 3);
        assert_eq!(t.country_name("is", "en"), Some("Iceland"));
        assert_eq!(t.country_name("ZZ", "en"), None);
    }

    #[test]
    fn coordinates_parse_with_minutes_and_seconds() {
        let (lat, lon) = parse_coordinates("+4230+00131").unwrap();
        assert!((lat - 42.5).abs() < 1e-9 && (lon - (1.0 + 31.0 / 60.0)).abs() < 1e-9);
        let (lat, lon) = parse_coordinates("+404251-0740023").unwrap();
        assert!(lat > 40.71 && lat < 40.72);
        assert!(lon < -74.0 && lon > -74.01);
        assert_eq!(parse_coordinates("garbage"), None);
        assert_eq!(parse_coordinates("+4290+00131"), None);
    }

    #[test]
    fn places_take_the_city_from_the_zone() {
        let t = table();
        assert_eq!(t.places().len(), 3);
        let ny = t.place_for_zone("America/New_York").unwrap();
        assert_eq!(ny.city, "New York");
        assert_eq!(ny.countries, vec!["US".to_string()]);
        let rv = t.place_for_zone("Atlantic/Reykjavik").unwrap();
        assert!(rv.latitude > 64.1 && rv.longitude < -21.8);
    }

    #[test]
    fn localised_country_names_override_the_table() {
        let mut t = table();
        t.set_country_name("is", "IS", "Ísland");
        assert_eq!(t.country_name("IS", "is"), Some("Ísland"));
        assert_eq!(t.country_name("IS", "en"), Some("Iceland"));
        assert_eq!(t.country_name("IS", "de"), Some("Iceland"));
    }

    #[test]
    fn search_ranks_city_prefix_before_word_and_country_matches() {
        let t = table();
        let hits = t.search("rey", "en", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].zone, "Atlantic/Reykjavik");
        assert_eq!(t.search("york", "en", 5)[0].city, "New York");
        assert_eq!(t.search("iceland", "en", 5)[0].city, "Reykjavik");
        assert!(t.search("", "en", 5).is_empty());
        assert!(t.search("zzz", "en", 5).is_empty());
    }

    #[test]
    fn fold_reduces_icelandic_letters() {
        assert_eq!(fold("Þórshöfn"), "thorshofn");
        assert_eq!(fold("New_York"), "new york");
        let mut t = table();
        t.set_country_name("is", "IS", "Ísland");
        assert_eq!(t.search("island", "is", 5)[0].city, "Reykjavik");
    }
}
