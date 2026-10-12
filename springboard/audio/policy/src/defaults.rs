//! Default devices per role and the jack rules. A chosen default stays chosen while its device is away. Headphones
//! plugged in become the default output; unplugged, the previous default returns and the media player pauses.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Output,
    Input,
    SoundEffects,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Output, Role::Input, Role::SoundEffects];
    fn key(self) -> &'static str {
        match self {
            Role::Output => "default_output",
            Role::Input => "default_input",
            Role::SoundEffects => "default_sound_effects",
        }
    }
    fn slot(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub uid: String,
    pub output: bool,
    pub input: bool,
    /// Built-in jack or headphone output (what the jack rule switches to).
    pub headphones: bool,
}

/// What the caller must do after an event.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    /// New (role, device uid) defaults to write to the system object.
    pub set: Vec<(Role, String)>,
    pub pause_media: bool,
    pub show_indicator: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Defaults {
    devices: Vec<Device>,
    chosen: [Option<String>; 3],
    /// The output the user had before headphones took over.
    before_jack: Option<String>,
    jack: bool,
}

impl Defaults {
    pub fn new() -> Self {
        Self::default()
    }

    fn can_take(d: &Device, role: Role) -> bool {
        match role {
            Role::Input => d.input,
            _ => d.output,
        }
    }

    /// The effective default: the chosen device if present, else the first able device.
    pub fn effective(&self, role: Role) -> Option<&str> {
        let ok = |d: &&Device| Self::can_take(d, role);
        self.chosen[role.slot()]
            .as_deref()
            .and_then(|u| self.devices.iter().filter(ok).find(|d| d.uid == u))
            .or_else(|| self.devices.iter().find(ok))
            .map(|d| d.uid.as_str())
    }

    pub fn choose(&mut self, role: Role, uid: &str) {
        self.chosen[role.slot()] = Some(uid.to_string());
    }

    fn snapshot(&self) -> Vec<Option<String>> {
        Role::ALL.iter().map(|r| self.effective(*r).map(String::from)).collect()
    }

    pub fn device_added(&mut self, d: Device) -> Actions {
        let before = self.snapshot();
        self.devices.retain(|x| x.uid != d.uid);
        self.devices.push(d);
        self.diff(before)
    }

    pub fn device_removed(&mut self, uid: &str) -> Actions {
        let before = self.snapshot();
        self.devices.retain(|x| x.uid != uid);
        self.diff(before)
    }

    fn diff(&self, before: Vec<Option<String>>) -> Actions {
        let mut a = Actions::default();
        for (i, r) in Role::ALL.iter().enumerate() {
            let now = self.effective(*r).map(String::from);
            if now != before[i] {
                if let Some(u) = now {
                    a.set.push((*r, u));
                }
            }
        }
        a
    }

    /// The headphone jack changed. In: the first headphone device becomes the output. Out: the earlier output
    /// returns (when still present) and the media player pauses.
    pub fn jack_changed(&mut self, plugged: bool) -> Actions {
        let mut a = Actions::default();
        if plugged == self.jack {
            return a;
        }
        self.jack = plugged;
        a.show_indicator = true;
        if plugged {
            let Some(h) = self.devices.iter().find(|d| d.headphones && d.output).map(|d| d.uid.clone()) else {
                return a;
            };
            let current = self.effective(Role::Output).map(String::from);
            if current.as_deref() == Some(h.as_str()) {
                // Already on the headphones: nothing to restore when they go.
                self.before_jack = None;
            } else {
                self.before_jack = current;
                self.chosen[Role::Output.slot()] = Some(h.clone());
                a.set.push((Role::Output, h));
            }
        } else {
            a.pause_media = true;
            if let Some(prev) = self.before_jack.take() {
                if self.devices.iter().any(|d| d.uid == prev) {
                    self.chosen[Role::Output.slot()] = Some(prev.clone());
                    a.set.push((Role::Output, prev));
                }
            }
        }
        a
    }

    /// `key=value` lines for the prefs file.
    pub fn save(&self) -> String {
        Role::ALL.iter().filter_map(|r| self.chosen[r.slot()].as_ref().map(|u| format!("{}={}\n", r.key(), u))).collect()
    }

    pub fn load(&mut self, text: &str) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=').filter(|(_, v)| !v.is_empty()) {
                if let Some(r) = Role::ALL.iter().find(|r| r.key() == k) {
                    self.chosen[r.slot()] = Some(v.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(uid: &str, h: bool) -> Device {
        Device { uid: uid.into(), output: true, input: false, headphones: h }
    }

    #[test]
    fn first_able_device_is_default_and_choice_survives_absence() {
        let mut d = Defaults::new();
        // The first output is the first able device for the sound-effects role too, so it takes that default as well.
        assert_eq!(
            d.device_added(dev("spk", false)).set,
            vec![(Role::Output, "spk".to_string()), (Role::SoundEffects, "spk".to_string())]
        );
        d.device_added(dev("usb", false));
        d.choose(Role::Output, "usb");
        assert_eq!(d.effective(Role::Output), Some("usb"));
        assert_eq!(d.device_removed("usb").set, vec![(Role::Output, "spk".to_string())]);
        assert_eq!(d.device_added(dev("usb", false)).set, vec![(Role::Output, "usb".to_string())]);
        assert_eq!(d.effective(Role::Input), None);
    }

    #[test]
    fn headphones_in_switch_and_out_restore_and_pause() {
        let mut d = Defaults::new();
        d.device_added(dev("spk", false));
        d.device_added(dev("hp", true));
        let a = d.jack_changed(true);
        assert_eq!(a.set, vec![(Role::Output, "hp".to_string())]);
        assert!(!a.pause_media && a.show_indicator);
        let a = d.jack_changed(false);
        assert_eq!(a.set, vec![(Role::Output, "spk".to_string())]);
        assert!(a.pause_media);
        assert!(d.jack_changed(false).set.is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut d = Defaults::new();
        d.choose(Role::Output, "a");
        d.choose(Role::SoundEffects, "b");
        assert_eq!(d.save(), "default_output=a\ndefault_sound_effects=b\n");
        let mut e = Defaults::new();
        e.load(&d.save());
        assert_eq!(e.save(), d.save());
    }

    #[test]
    fn input_takes_only_input_devices_and_effects_follow_outputs() {
        let mut d = Defaults::new();
        let a = d.device_added(dev("spk", false));
        assert_eq!(a.set, vec![(Role::Output, "spk".to_string()), (Role::SoundEffects, "spk".to_string())]);
        let mic = Device { uid: "mic".into(), output: false, input: true, headphones: false };
        assert_eq!(d.device_added(mic).set, vec![(Role::Input, "mic".to_string())]);
        d.choose(Role::Input, "spk");
        assert_eq!(d.effective(Role::Input), Some("mic"), "an output-only device cannot be the input");
        assert!(d.device_removed("nothing").set.is_empty());
    }

    #[test]
    fn removing_the_last_device_sets_nothing() {
        let mut d = Defaults::new();
        d.device_added(dev("spk", false));
        assert!(d.device_removed("spk").set.is_empty());
        assert_eq!(d.effective(Role::Output), None);
    }

    #[test]
    fn jack_without_a_headphone_device_only_shows_the_indicator() {
        let mut d = Defaults::new();
        d.device_added(dev("spk", false));
        let a = d.jack_changed(true);
        assert!(a.set.is_empty() && a.show_indicator && !a.pause_media);
        let a = d.jack_changed(false);
        assert!(a.set.is_empty() && a.pause_media);
    }

    #[test]
    fn jack_out_does_not_restore_the_headphones_themselves() {
        let mut d = Defaults::new();
        d.device_added(dev("spk", false));
        d.device_added(dev("hp", true));
        d.choose(Role::Output, "hp");
        assert!(d.jack_changed(true).set.is_empty());
        let a = d.jack_changed(false);
        assert!(a.set.is_empty() && a.pause_media);
    }

    #[test]
    fn jack_out_skips_a_previous_output_that_went_away() {
        let mut d = Defaults::new();
        d.device_added(dev("usb", false));
        d.device_added(dev("hp", true));
        d.jack_changed(true);
        d.device_removed("usb");
        let a = d.jack_changed(false);
        assert!(a.set.is_empty());
        assert_eq!(d.effective(Role::Output), Some("hp"));
    }

    #[test]
    fn load_ignores_unknown_keys_and_empty_values() {
        let mut d = Defaults::new();
        d.load("default_output=\nvolume=3\nnonsense\ndefault_input=mic\n");
        assert_eq!(d.save(), "default_input=mic\n");
    }
}
