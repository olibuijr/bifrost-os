//! Settings > Sound rows built from the audio devices the shell can see (the ALSA cards and PCMs in `/proc/asound`, the
//! headphone jack route and the connected Bluetooth audio device: the same sources as the `audio-devices` service), the
//! live output level, and the menu-bar input indicator flag. `view` is pure; `read_view` reads `/proc/asound` (two
//! small procfs files) when the Sound page opens. Nothing here opens a device.

use super::list::{Ends, Group, Item, Tint};
use super::pages::{Action, Pick, Slider};
use std::sync::atomic::{AtomicBool, Ordering};

pub const PREF_OUTPUT: &str = "audio_default_output";
pub const PREF_INPUT: &str = "audio_default_input";
/// Settings > Sound > Play sound effects through: the output's UID, or empty for the selected output device
pub const PREF_EFFECTS: &str = "audio_default_sound_effects";

/// The first choice of "Play sound effects through": wherever the selected output plays.
pub const SELECTED_OUTPUT: &str = "Selected Sound Output Device";

/// One listed device: display name, the server's UID and its type ("Built-in", "USB", "HDMI", "Bluetooth").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceRow {
    pub name: String,
    pub uid: String,
    pub kind: String,
}

/// What the page shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundView {
    pub outputs: Vec<DeviceRow>,
    pub inputs: Vec<DeviceRow>,
    /// Index into `outputs` / `inputs` of the current default.
    pub output: Option<usize>,
    pub input: Option<usize>,
    /// Index into `outputs` of the device sound effects play through; None follows the selected output.
    pub effects: Option<usize>,
    /// The input level, 0..=1 (peak of the default input).
    pub level: f32,
    /// The output volume, 0..=1: the live level the Control Center slider and the volume keys set.
    pub volume: f32,
}

pub static NO_SOUND: SoundView = SoundView { outputs: Vec::new(), inputs: Vec::new(), output: None, input: None, effects: None, level: 0.0, volume: 0.0 };

impl SoundView {
    /// The chosen entry of "Play sound effects through": 0 is the selected output device, n the (n-1)th output.
    pub fn effects_choice(&self) -> usize {
        self.effects.filter(|&k| k < self.outputs.len()).map_or(0, |k| k + 1)
    }

    /// The choices of "Play sound effects through": the selected output device, then every output.
    pub fn effects_choices(&self) -> Vec<String> {
        std::iter::once(SELECTED_OUTPUT.to_string()).chain(self.outputs.iter().map(|d| d.name.clone())).collect()
    }

    /// Selects the defaults named by the saved UIDs (`PREF_OUTPUT`, `PREF_INPUT`, `PREF_EFFECTS`); a device that is
    /// not there leaves the first one selected.
    pub fn choose(&mut self, output: Option<&str>, input: Option<&str>, effects: Option<&str>) {
        let at = |list: &[DeviceRow], uid: Option<&str>| uid.and_then(|u| list.iter().position(|d| d.uid == u));
        self.output = at(&self.outputs, output).or((!self.outputs.is_empty()).then_some(0));
        self.input = at(&self.inputs, input).or((!self.inputs.is_empty()).then_some(0));
        self.effects = at(&self.outputs, effects.filter(|u| !u.is_empty()));
    }
}

/// The groups of Settings > Sound under Sound Effects: the output devices and the output volume, then the input
/// devices and the input level.
pub fn groups(v: &SoundView) -> Vec<Group> {
    let mut g = Vec::new();
    let outputs: Vec<Item> = if v.outputs.is_empty() {
        vec![Item::new("No output devices found").tint(Tint::Dim)]
    } else {
        v.outputs.iter().enumerate().map(|(i, d)| Item::new(d.name.as_str()).value(d.kind.as_str()).check(v.output == Some(i), Action::Pick(Pick::SoundOutput(i)))).collect()
    };
    g.push(Group::new(outputs).header("OUTPUT").footer("Choose where sound plays. Plugging in headphones switches to them."));
    g.push(Group::new(vec![Item::slider(Ends::Volume, v.volume.clamp(0.0, 1.0), Action::Slider(Slider::Volume))]).header("OUTPUT VOLUME"));
    let mut inputs: Vec<Item> = if v.inputs.is_empty() {
        vec![Item::new("No input devices found").tint(Tint::Dim)]
    } else {
        v.inputs.iter().enumerate().map(|(i, d)| Item::new(d.name.as_str()).value(d.kind.as_str()).check(v.input == Some(i), Action::Pick(Pick::SoundInput(i)))).collect()
    };
    if !v.inputs.is_empty() {
        inputs.push(Item::new("Input Level").value(level_label(v.level)));
    }
    g.push(Group::new(inputs).header("INPUT"));
    g
}

/// The input level as a percentage.
pub fn level_label(level: f32) -> String {
    format!("{:.0}%", level.clamp(0.0, 1.0) * 100.0)
}

/// The pref (key, value) to store when the user picks output or input `k`.
pub fn pick_pref(v: &SoundView, input: bool, k: usize) -> Option<(&'static str, String)> {
    let list = if input { &v.inputs } else { &v.outputs };
    list.get(k).map(|d| (if input { PREF_INPUT } else { PREF_OUTPUT }, d.uid.clone()))
}

/// The pref value to store when "Play sound effects through" is set to choice `k` (0 is the selected output: empty).
pub fn effects_pref(v: &SoundView, k: usize) -> String {
    k.checked_sub(1).and_then(|i| v.outputs.get(i)).map_or_else(String::new, |d| d.uid.clone())
}

/// One PCM device of `/proc/asound/pcm`: "00-00: ALC892 Analog : ALC892 Analog : playback 1 : capture 1".
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub card: u32,
    pub device: u32,
    pub name: String,
    pub playback: bool,
    pub capture: bool,
}

/// The PCM devices listed in `/proc/asound/pcm`; lines that do not parse are skipped.
pub fn parse_pcm(text: &str) -> Vec<Pcm> {
    text.lines()
        .filter_map(|line| {
            let (id, rest) = line.split_once(':')?;
            let (card, device) = id.trim().split_once('-')?;
            let fields: Vec<&str> = rest.split(" : ").map(str::trim).collect();
            let name = fields.first()?.to_string();
            let has = |what: &str| fields.iter().any(|f| f.starts_with(what));
            Some(Pcm { card: card.parse().ok()?, device: device.parse().ok()?, name, playback: has("playback"), capture: has("capture") })
        })
        .collect()
}

/// The cards of `/proc/asound/cards` as (number, driver, long name): " 0 [PCH ]: HDA-Intel - HDA Intel PCH".
pub fn parse_cards(text: &str) -> Vec<(u32, String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (num, rest) = line.split_once(' ')?;
            let num: u32 = num.parse().ok()?;
            let (_, desc) = rest.split_once("]:")?;
            let (driver, long) = desc.trim().split_once(" - ").unwrap_or((desc.trim(), desc.trim()));
            Some((num, driver.trim().to_string(), long.trim().to_string()))
        })
        .collect()
}

/// The page's devices from the ALSA lists, the headphone jack and the connected Bluetooth audio device. The first
/// built-in analog output is the speaker (or the headphones while they are plugged in) and the first built-in input
/// the microphone, under the `audio-devices` service's UIDs; the others keep their ALSA names under "alsa:CARD-DEV".
pub fn view(pcm: &str, cards: &str, headphones: bool, bluetooth: Option<&str>) -> SoundView {
    use super::services::audio_devices::{BLUETOOTH, HEADPHONES, MICROPHONE, SPEAKER};
    let cards = parse_cards(cards);
    let card = |n: u32| cards.iter().find(|c| c.0 == n);
    let kind = |p: &Pcm| {
        let usb = card(p.card).is_some_and(|c| c.1.eq_ignore_ascii_case("USB-Audio"));
        let upper = p.name.to_uppercase();
        if usb {
            "USB"
        } else if upper.contains("HDMI") || upper.contains("DISPLAYPORT") {
            "HDMI"
        } else if upper.contains("DIGITAL") || upper.contains("SPDIF") {
            "Digital Out"
        } else {
            "Built-in"
        }
    };
    // a USB device names itself by its card ("Blue Snowball"); a PCM's own name is generic there ("USB Audio")
    let name = |p: &Pcm| match card(p.card) {
        Some((_, driver, long)) if driver.eq_ignore_ascii_case("USB-Audio") => long.clone(),
        _ => p.name.clone(),
    };
    let pcms = parse_pcm(pcm);
    let mut v = SoundView::default();
    let mut speaker = false;
    for p in pcms.iter().filter(|p| p.playback) {
        let k = kind(p);
        let row = if k == "Built-in" && !speaker {
            speaker = true;
            if headphones {
                DeviceRow { name: "Headphones".into(), uid: HEADPHONES.into(), kind: "Headphone Port".into() }
            } else {
                DeviceRow { name: "Speakers".into(), uid: SPEAKER.into(), kind: k.into() }
            }
        } else {
            DeviceRow { name: name(p), uid: format!("alsa:{}-{}", p.card, p.device), kind: k.into() }
        };
        v.outputs.push(row);
    }
    if !speaker {
        // no ALSA list (or no analog output in it): the route still says where sound plays
        let row = if headphones { DeviceRow { name: "Headphones".into(), uid: HEADPHONES.into(), kind: "Headphone Port".into() } } else { DeviceRow { name: "Speakers".into(), uid: SPEAKER.into(), kind: "Built-in".into() } };
        v.outputs.insert(0, row);
    }
    if let Some(bt) = bluetooth {
        v.outputs.push(DeviceRow { name: bt.to_string(), uid: BLUETOOTH.into(), kind: "Bluetooth".into() });
    }
    let mut mic = false;
    for p in pcms.iter().filter(|p| p.capture) {
        let k = kind(p);
        let row = if k == "Built-in" && !mic {
            mic = true;
            DeviceRow { name: "Microphone".into(), uid: MICROPHONE.into(), kind: k.into() }
        } else {
            DeviceRow { name: name(p), uid: format!("alsa:{}-{}", p.card, p.device), kind: k.into() }
        };
        v.inputs.push(row);
    }
    v.choose(None, None, None);
    v
}

/// `view` from the system's `/proc/asound` (missing files are empty lists).
pub fn read_view(headphones: bool, bluetooth: Option<&str>) -> SoundView {
    let pcm = std::fs::read_to_string("/proc/asound/pcm").unwrap_or_default();
    let cards = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    view(&pcm, &cards, headphones, bluetooth)
}

static INPUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The audio service sets this while any input stream or tap runs; the menu bar draws the indicator from it.
pub fn set_input_active(on: bool) {
    INPUT_ACTIVE.store(on, Ordering::Relaxed);
}

pub fn input_active() -> bool {
    INPUT_ACTIVE.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_of() -> SoundView {
        let d = |n: &str, u: &str| DeviceRow { name: n.into(), uid: u.into(), kind: "Built-in".into() };
        SoundView { outputs: vec![d("Speakers", "spk"), d("Headphones", "hp")], inputs: vec![d("Mic", "mic")], output: Some(1), input: Some(0), effects: None, level: 0.256, volume: 0.644 }
    }

    const PCM: &str = "00-00: ALC892 Analog : ALC892 Analog : playback 1 : capture 1\n00-01: ALC892 Digital : ALC892 Digital : playback 1\n00-03: HDMI 0 : HDMI 0 : playback 1\n01-00: USB Audio : USB Audio : capture 1\n";
    const CARDS: &str = " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n                      HDA Intel PCH at 0xf7f10000 irq 32\n 1 [Snowball       ]: USB-Audio - Blue Snowball\n                      BLUE Blue Snowball at usb-0000:00:14.0-2, full speed\n";

    #[test]
    fn an_empty_view_says_there_are_no_devices_and_keeps_the_volume() {
        let g = groups(&NO_SOUND);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].items[0].label, "No output devices found");
        assert_eq!(g[2].items[0].label, "No input devices found");
    }

    #[test]
    fn groups_list_devices_the_live_volume_and_level() {
        let g = groups(&view_of());
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].items.len(), 2);
        assert_eq!(g[0].items[0].value, "Built-in", "the device's type sits beside its name");
        assert_eq!(g[1].items[0].body, super::super::list::Body::Slider { ends: Ends::Volume, value: 0.644 }, "the output volume is the live level");
        assert_eq!(g[2].items.len(), 2);
        assert_eq!(g[2].items[1].label.as_str(), "Input Level");
        assert_eq!(level_label(0.256), "26%");
        assert_eq!(level_label(3.0), "100%");
    }

    #[test]
    fn pick_maps_to_pref() {
        let v = view_of();
        assert_eq!(pick_pref(&v, false, 1), Some((PREF_OUTPUT, "hp".to_string())));
        assert_eq!(pick_pref(&v, true, 0), Some((PREF_INPUT, "mic".to_string())));
        assert_eq!(pick_pref(&v, true, 5), None);
        assert_eq!(effects_pref(&v, 0), "", "the selected output device is stored as empty");
        assert_eq!(effects_pref(&v, 2), "hp");
    }

    #[test]
    fn alsa_lists_become_named_devices() {
        let v = view(PCM, CARDS, false, Some("Pods"));
        let outs: Vec<(&str, &str)> = v.outputs.iter().map(|d| (d.name.as_str(), d.kind.as_str())).collect();
        assert_eq!(outs, vec![("Speakers", "Built-in"), ("ALC892 Digital", "Digital Out"), ("HDMI 0", "HDMI"), ("Pods", "Bluetooth")]);
        let ins: Vec<(&str, &str)> = v.inputs.iter().map(|d| (d.name.as_str(), d.kind.as_str())).collect();
        assert_eq!(ins, vec![("Microphone", "Built-in"), ("Blue Snowball", "USB")]);
        assert_eq!((v.output, v.input), (Some(0), Some(0)));
        let jack = view(PCM, CARDS, true, None);
        assert_eq!(jack.outputs[0].name, "Headphones", "the jack's route names the analog output");
        let none = view("", "", false, None);
        assert_eq!(none.outputs.len(), 1, "without ALSA the route still lists the speaker");
        assert!(none.inputs.is_empty());
    }

    #[test]
    fn saved_choices_select_their_devices() {
        let mut v = view(PCM, CARDS, false, None);
        v.choose(Some("alsa:0-3"), Some("alsa:1-0"), Some("alsa:0-1"));
        assert_eq!((v.output, v.input, v.effects), (Some(2), Some(1), Some(1)));
        assert_eq!(v.effects_choice(), 2);
        assert_eq!(v.effects_choices()[0], SELECTED_OUTPUT);
        v.choose(Some("gone"), None, Some(""));
        assert_eq!((v.output, v.effects), (Some(0), None), "a missing device leaves the first one selected");
    }

    #[test]
    fn indicator_flag() {
        set_input_active(true);
        assert!(input_active());
        set_input_active(false);
        assert!(!input_active());
    }
}
