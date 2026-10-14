//! Shell-provided App Intents for System Settings: Wi-Fi power and scan, and
//! setting the wallpaper by id. They sit beside the app intents of `intents.rs`:
//! Search can list them from [`catalogue`], and [`parse`] turns a request into a
//! [`SettingsIntent`] whose [`SettingsIntent::into_effect`] is the same `Effect`
//! the Settings pane emits, so the shell applies it through its existing path.

use super::{Effect, WifiAction};

/// One request to the Settings intents, with its parameters already checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsIntent {
    /// Wi-Fi radio on or off
    WifiPower(bool),
    /// ask the radio for a network scan
    WifiScan,
    /// the home screen wallpaper becomes the packed still with this id
    SetWallpaper(String),
}

/// A catalogue entry: the id a caller sends, a title for Search, and the parameter names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub id: &'static str,
    pub title: &'static str,
    /// parameters without a `?`: every one is required
    pub params: &'static [&'static str],
}

static CATALOGUE: [Info; 3] = [
    Info { id: "wifi-power", title: "Set Wi-Fi Power", params: &["on"] },
    Info { id: "wifi-scan", title: "Scan for Wi-Fi Networks", params: &[] },
    Info { id: "wallpaper-set", title: "Set Wallpaper", params: &["id"] },
];

/// Longest wallpaper id accepted (ids name `wallpapers/<id>.raw`).
const WALLPAPER_ID_MAX: usize = 64;

/// Every Settings intent the shell offers.
pub fn catalogue() -> &'static [Info] {
    &CATALOGUE
}

/// The catalogue entry with this id.
pub fn find(id: &str) -> Option<&'static Info> {
    CATALOGUE.iter().find(|info| info.id == id)
}

/// Checks a request: `id` names an entry, and `params` (`(name, value)` pairs)
/// carry every required parameter with a valid value.
pub fn parse(id: &str, params: &[(&str, &str)]) -> Result<SettingsIntent, String> {
    let info = find(id).ok_or_else(|| format!("no settings intent {id}"))?;
    for (name, _) in params {
        if !info.params.iter().any(|p| *p == *name) {
            return Err(format!("{id} takes no {name}"));
        }
    }
    match id {
        "wifi-power" => Ok(SettingsIntent::WifiPower(parse_bool(&need(id, params, "on")?)?)),
        "wifi-scan" => Ok(SettingsIntent::WifiScan),
        "wallpaper-set" => {
            let wid = need(id, params, "id")?;
            if valid_wallpaper_id(&wid) {
                Ok(SettingsIntent::SetWallpaper(wid))
            } else {
                Err(format!("bad wallpaper id {wid:?}"))
            }
        }
        _ => Err(format!("no settings intent {id}")),
    }
}

/// The value of parameter `name`, which the request must carry.
fn need(id: &str, params: &[(&str, &str)], name: &str) -> Result<String, String> {
    params.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()).ok_or_else(|| format!("{id} needs {name}"))
}

impl SettingsIntent {
    /// The shell effect this request performs.
    pub fn into_effect(self) -> Effect {
        match self {
            SettingsIntent::WifiPower(on) => Effect::Wifi(WifiAction::Enable(on)),
            SettingsIntent::WifiScan => Effect::Wifi(WifiAction::Scan),
            SettingsIntent::SetWallpaper(id) => Effect::Wallpaper(id),
        }
    }
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "true" | "1" | "on" => Ok(true),
        "false" | "0" | "off" => Ok(false),
        _ => Err(format!("not a switch value: {v:?}")),
    }
}

/// A wallpaper id is non-empty, short, and made of ASCII letters, digits, `-` and `_`.
fn valid_wallpaper_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= WALLPAPER_ID_MAX && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_lists_three_intents_with_unique_ids() {
        let ids: Vec<&str> = catalogue().iter().map(|i| i.id).collect();
        assert_eq!(ids, ["wifi-power", "wifi-scan", "wallpaper-set"]);
        assert_eq!(find("wifi-scan").map(|i| i.title), Some("Scan for Wi-Fi Networks"));
        assert!(find("wifi-off").is_none());
    }

    #[test]
    fn wifi_power_takes_a_switch_value() {
        assert_eq!(parse("wifi-power", &[("on", "true")]), Ok(SettingsIntent::WifiPower(true)));
        assert_eq!(parse("wifi-power", &[("on", "0")]), Ok(SettingsIntent::WifiPower(false)));
        assert!(parse("wifi-power", &[("on", "maybe")]).is_err());
        assert!(parse("wifi-power", &[]).is_err());
    }

    #[test]
    fn scan_takes_no_parameters() {
        assert_eq!(parse("wifi-scan", &[]), Ok(SettingsIntent::WifiScan));
        assert!(parse("wifi-scan", &[("on", "true")]).is_err());
    }

    #[test]
    fn wallpaper_id_is_checked() {
        assert_eq!(parse("wallpaper-set", &[("id", "waterfall_idyllic")]), Ok(SettingsIntent::SetWallpaper("waterfall_idyllic".into())));
        assert!(parse("wallpaper-set", &[("id", "")]).is_err());
        assert!(parse("wallpaper-set", &[("id", "../etc/passwd")]).is_err());
        assert!(parse("wallpaper-set", &[("id", &"a".repeat(65))]).is_err());
        assert!(parse("wallpaper-set", &[]).is_err());
    }

    #[test]
    fn unknown_intents_are_refused() {
        assert_eq!(parse("reboot", &[]), Err("no settings intent reboot".into()));
    }

    #[test]
    fn requests_map_to_the_effects_the_pane_emits() {
        assert_eq!(SettingsIntent::WifiPower(false).into_effect(), Effect::Wifi(WifiAction::Enable(false)));
        assert_eq!(SettingsIntent::WifiScan.into_effect(), Effect::Wifi(WifiAction::Scan));
        assert_eq!(SettingsIntent::SetWallpaper("kirkjufell_dusk".into()).into_effect(), Effect::Wallpaper("kirkjufell_dusk".into()));
    }
}
