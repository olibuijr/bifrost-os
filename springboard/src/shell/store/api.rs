//! The store catalogue: `GET /api/v1/apps.json`, parsed, cached on disk for
//! offline use, and compared against what is installed.

use super::install::{parse_semver, InstalledApp};
use crate::shell::kinds::Kind;
use super::json;
use bifrost_ui::purchase::Price;
use std::path::Path;

/// The catalogue, below the store host (`net::STORE_BASE`).
pub const CATALOG_PATH: &str = "/api/v1/apps.json";
pub const CACHE_PATH: &str = "/var/lib/springboard/store-cache.json";

/// The age ratings the store uses, lowest first.
pub const AGE_RATINGS: [&str; 5] = ["4+", "9+", "12+", "16+", "18+"];

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StoreApp {
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub description: String,
    pub category: String,
    pub size: u64,
    pub sha256: String,
    pub icon: String,
    pub package: String,
    pub min_shell: String,
    /// `"app"` (default) or `"system"`
    pub kind: String,
    /// release notes, newest first (empty for entries the server has no notes for)
    pub changelog: Vec<Release>,
    /// the price in the store's currency; `None` for a free app (and for an entry that gives no currency)
    pub price: Option<Price>,
    /// one of `AGE_RATINGS`, or empty when the entry is unrated or gives an unknown rating
    pub age_rating: String,
}

/// One released version and what changed in it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, or empty
    pub date: String,
    pub notes: Vec<String>,
}

/// Reads a `changelog` array; entries without a version are skipped, absence gives an empty list.
pub fn parse_changelog(v: Option<&json::Json>) -> Vec<Release> {
    let Some(list) = v.and_then(|c| c.as_array()) else { return vec![] };
    list.iter()
        .filter_map(|r| {
            let version = r.str_field("version")?;
            let notes = r.get("notes").and_then(|n| n.as_array()).map(|n| n.iter().filter_map(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
            Some(Release { version, date: r.str_field("date").unwrap_or_default(), notes })
        })
        .collect()
}

/// "Today", "Yesterday", "3d ago" (under a week), else "5 Oct 2026"; `today` is days since the epoch.
pub fn relative_date(date: &str, today: i64) -> String {
    let mut it = date.split('-').map(|p| p.parse::<i64>());
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (it.next(), it.next(), it.next()) else { return date.to_string() };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return date.to_string();
    }
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match today - crate::bare::timezone::days_from_civil(y, m as u32, d as u32) {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        n @ 2..=6 => format!("{n}d ago"),
        _ => format!("{d} {} {y}", MONTHS[(m - 1) as usize]),
    }
}

impl StoreApp {
    /// A `system` data package (the "System Component" rows).
    pub fn is_system(&self) -> bool {
        Kind::of(&self.kind) == Kind::System
    }

    /// A paid app: the store's price is above zero. The shell has no way to buy one yet, so it is never installed.
    pub fn is_paid(&self) -> bool {
        self.price.as_ref().is_some_and(|p| p.minor > 0)
    }

    /// "Free" or the price as the customer sees it, e.g. "12.50 EUR".
    pub fn price_text(&self) -> String {
        match self.price.as_ref() {
            Some(p) if self.is_paid() => p.display(),
            _ => "Free".into(),
        }
    }

    /// The age rating, or "-" when the store gives none.
    pub fn age_text(&self) -> String {
        if self.age_rating.is_empty() { "-".into() } else { self.age_rating.clone() }
    }

    /// `system`, `system-app` or `service`: installed in the background, never offered a GET or Remove.
    pub fn is_preinstalled(&self) -> bool {
        Kind::of(&self.kind).preinstalled()
    }

    /// An `bifrostd` service: installed in the background and invisible in the App Store.
    pub fn is_service(&self) -> bool {
        Kind::of(&self.kind) == Kind::Service
    }
}

/// Parses the catalogue; entries missing a required field are skipped.
pub fn parse_catalog(text: &str) -> Result<Vec<StoreApp>, String> {
    let root = json::parse(text)?;
    let list = root.get("apps").and_then(|a| a.as_array()).ok_or("catalogue has no \"apps\" array")?;
    let mut out = vec![];
    for a in list {
        let (Some(id), Some(name), Some(version), Some(package)) = (a.str_field("id"), a.str_field("name"), a.str_field("version"), a.str_field("package")) else { continue };
        let sha256 = a.str_field("sha256").unwrap_or_default().to_ascii_lowercase();
        if parse_semver(&version).is_none() || sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let summary = a.str_field("summary").unwrap_or_default();
        out.push(StoreApp {
            id,
            name,
            version,
            description: a.str_field("description").unwrap_or_else(|| summary.clone()),
            summary,
            category: a.str_field("category").unwrap_or_else(|| "Utilities".into()),
            size: a.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
            sha256,
            icon: a.str_field("icon").unwrap_or_default(),
            package,
            min_shell: a.str_field("min_shell").unwrap_or_else(|| "0.0.0".into()),
            kind: a.str_field("kind").unwrap_or_else(|| "app".into()),
            changelog: parse_changelog(a.get("changelog")),
            price: parse_price(a),
            age_rating: age_rating_of(&a.str_field("age_rating").unwrap_or_default()),
        });
    }
    Ok(out)
}

/// Reads `price` (minor units), `currency` (ISO 4217) and `fraction_digits` (default 2) of a catalogue entry.
/// Without a currency there is no price to show, so the entry is free.
fn parse_price(a: &json::Json) -> Option<Price> {
    let minor = i64::try_from(a.get("price")?.as_u64()?).ok()?;
    let currency = a.str_field("currency")?;
    let digits = a.get("fraction_digits").and_then(|d| d.as_u64()).unwrap_or(2) as u32;
    Some(Price::new(minor, digits, &currency))
}

/// The rating when it is one of `AGE_RATINGS` (surrounding spaces ignored), else empty.
pub fn age_rating_of(v: &str) -> String {
    let v = v.trim();
    if AGE_RATINGS.contains(&v) { v.to_string() } else { String::new() }
}

/// Whether the store's version is newer than the installed one.
pub fn update_available(installed: &str, store: &str) -> bool {
    matches!((parse_semver(installed), parse_semver(store)), (Some(i), Some(s)) if s > i)
}

/// The installed apps that have a newer store version; services update silently and are left out.
pub fn updates<'a>(catalog: &'a [StoreApp], installed: &[InstalledApp]) -> Vec<&'a StoreApp> {
    catalog.iter().filter(|s| !s.is_service() && installed.iter().any(|i| i.id == s.id && update_available(&i.version, &s.version))).collect()
}

/// The packages to install or update in the background: every `kind = "system"`, `"system-app"` or `"service"` entry
/// that is not installed yet, or has a newer version, and that this shell can run.
pub fn system_plan<'a>(catalog: &'a [StoreApp], installed: &[InstalledApp], shell: &str) -> Vec<&'a StoreApp> {
    let have = parse_semver(shell).unwrap_or((0, 0, 0));
    catalog
        .iter()
        .filter(|s| s.is_preinstalled() && parse_semver(&s.min_shell).is_some_and(|m| m <= have))
        .filter(|s| match installed.iter().find(|i| i.id == s.id) {
            Some(i) => update_available(&i.version, &s.version),
            None => true,
        })
        .collect()
}

/// Fetches the catalogue over HTTPS; a good answer replaces the cache.
pub fn fetch(cache: &Path) -> Result<Vec<StoreApp>, String> {
    let body = super::net::get(&super::net::absolute("/api/v1/pc/apps.json"), 2_000_000).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let apps = parse_catalog(&text)?;
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = cache.with_extension("tmp");
    if std::fs::write(&tmp, &text).is_ok() {
        let _ = std::fs::rename(tmp, cache);
    }
    Ok(apps)
}

pub fn load_cache(cache: &Path) -> Option<Vec<StoreApp>> {
    parse_catalog(&std::fs::read_to_string(cache).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUM: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn doc() -> String {
        format!(
            r#"{{"apps":[{{"id":"is.olibuijr.calc","name":"Calc","version":"1.2.0","summary":"Adds","category":"Utilities","size":1234,"sha256":"{SUM}","icon":"/store/is.olibuijr.calc/icon.png","package":"/store/is.olibuijr.calc/is.olibuijr.calc-1.2.0.aap","min_shell":"0.1.0"}},{{"id":"broken","name":"x"}},{{"id":"bad.sha","name":"B","version":"1.0.0","sha256":"zz","package":"/p"}}]}}"#
        )
    }

    #[test]
    fn catalogue_parses_and_skips_broken_entries() {
        let apps = parse_catalog(&doc()).unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!((apps[0].name.as_str(), apps[0].size, apps[0].min_shell.as_str()), ("Calc", 1234, "0.1.0"));
        assert_eq!(apps[0].description, "Adds");
        assert!(parse_catalog("{}").is_err() && parse_catalog("nope").is_err());
    }

    #[test]
    fn changelog_parses_and_is_optional() {
        let text = format!(
            r#"{{"apps":[{{"id":"a","name":"A","version":"1.1.0","sha256":"{SUM}","package":"/p","changelog":[{{"version":"1.1.0","date":"2026-10-05","notes":["Fixed x"," Added y "]}},{{"date":"2026-01-01"}},{{"version":"1.0.0"}}]}},{{"id":"b","name":"B","version":"1.0.0","sha256":"{SUM}","package":"/q"}}]}}"#
        );
        let apps = parse_catalog(&text).unwrap();
        assert_eq!(apps[0].changelog.len(), 2);
        assert_eq!(apps[0].changelog[0], Release { version: "1.1.0".into(), date: "2026-10-05".into(), notes: vec!["Fixed x".into(), "Added y".into()] });
        assert!(apps[0].changelog[1].notes.is_empty() && apps[1].changelog.is_empty());
    }

    #[test]
    fn price_and_age_rating_come_from_the_catalogue() {
        let text = format!(
            r#"{{"apps":[{{"id":"a","name":"A","version":"1.0.0","sha256":"{SUM}","package":"/p","price":1250,"currency":"EUR","age_rating":"12+"}},{{"id":"b","name":"B","version":"1.0.0","sha256":"{SUM}","package":"/q","price":990,"currency":"ISK","fraction_digits":0,"age_rating":"7+"}},{{"id":"c","name":"C","version":"1.0.0","sha256":"{SUM}","package":"/r"}},{{"id":"d","name":"D","version":"1.0.0","sha256":"{SUM}","package":"/s","price":500}}]}}"#
        );
        let apps = parse_catalog(&text).unwrap();
        assert_eq!(apps[0].price, Some(Price::new(1250, 2, "EUR")));
        assert!(apps[0].is_paid());
        assert_eq!((apps[0].price_text(), apps[0].age_text()), ("12.50 EUR".to_string(), "12+".to_string()));
        assert_eq!((apps[1].price_text(), apps[1].age_text()), ("990 ISK".to_string(), "-".to_string()), "7+ is no rating the store uses");
        assert_eq!((apps[2].is_paid(), apps[2].price_text(), apps[2].age_text()), (false, "Free".to_string(), "-".to_string()));
        assert!(!apps[3].is_paid(), "a price with no currency is no price");
        assert_eq!(age_rating_of(" 18+ "), "18+");
    }

    #[test]
    fn relative_dates() {
        let today = crate::bare::timezone::days_from_civil(2026, 10, 5);
        assert_eq!(relative_date("2026-10-05", today), "Today");
        assert_eq!(relative_date("2026-10-04", today), "Yesterday");
        assert_eq!(relative_date("2026-10-02", today), "3d ago");
        assert_eq!(relative_date("2026-09-28", today), "28 Sep 2026");
        assert_eq!(relative_date("2026-10-06", today), "6 Oct 2026");
        assert_eq!(relative_date("soon", today), "soon");
        assert_eq!(crate::bare::timezone::days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn update_comparison_is_semver_not_string_order() {
        assert!(update_available("1.2.0", "1.10.0"));
        assert!(!update_available("1.10.0", "1.2.0") && !update_available("1.0.0", "1.0.0"));
        assert!(!update_available("junk", "1.0.0"));
    }

    #[test]
    fn updates_lists_only_installed_apps_with_newer_versions() {
        let apps = parse_catalog(&doc()).unwrap();
        let mk = |v: &str| InstalledApp { id: "is.olibuijr.calc".into(), name: "Calc".into(), version: v.into(), ..Default::default() };
        assert_eq!(updates(&apps, &[mk("1.1.0")]).len(), 1);
        assert!(updates(&apps, &[mk("1.2.0")]).is_empty() && updates(&apps, &[]).is_empty());
    }

    #[test]
    fn cache_round_trips() {
        let p = std::env::temp_dir().join(format!("apps-store-cache-{}.json", std::process::id()));
        std::fs::write(&p, doc()).unwrap();
        assert_eq!(load_cache(&p).unwrap().len(), 1);
        std::fs::remove_file(&p).unwrap();
        assert!(load_cache(&p).is_none());
    }

    #[test]
    fn preinstall_plan_takes_missing_and_outdated_system_packages_only() {
        let sys = |id: &str, v: &str, kind: &str, min: &str| StoreApp { id: id.into(), version: v.into(), kind: kind.into(), min_shell: min.into(), ..Default::default() };
        let cat = vec![sys("is.o.kb", "1.1.0", "system", "0.1.0"), sys("is.o.app", "1.0.0", "app", "0.1.0"), sys("is.o.new", "1.0.0", "system", "9.0.0")];
        let ids = |p: Vec<&StoreApp>| p.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        let mk = |v: &str| InstalledApp { id: "is.o.kb".into(), version: v.into(), kind: "system".into(), ..Default::default() };
        assert_eq!(ids(system_plan(&cat, &[], "0.1.0")), ["is.o.kb"], "missing: install; apps and too-new packages: skip");
        assert_eq!(ids(system_plan(&cat, &[mk("1.0.0")], "0.1.0")), ["is.o.kb"], "outdated: update");
        assert!(system_plan(&cat, &[mk("1.1.0")], "0.1.0").is_empty(), "current: nothing");
        assert_eq!(parse_catalog(&doc()).unwrap()[0].kind, "app");
    }
}
