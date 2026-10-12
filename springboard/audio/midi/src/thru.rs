//! MIDI thru connections: play-through of note and control events with value maps and control transforms.
//!
//! Pure: no port is opened and no event is sent. The caller feeds each incoming event to
//! [`ThruConnection::process`] and forwards whatever comes back. Values are 7-bit (0 to 127), channels are 0 to 15,
//! and the names follow the official framework's thru connection parameters (`MIDIValueMap`,
//! `MIDIControlTransform`, `MIDIThruConnectionParams`).

/// A 7-bit lookup table: each input value 0 to 127 maps to an output value 0 to 127.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueMap {
    table: [u8; 128],
}

impl Default for ValueMap {
    fn default() -> Self {
        Self::identity()
    }
}

impl ValueMap {
    /// Every value maps to itself.
    pub fn identity() -> Self {
        let mut table = [0u8; 128];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = i as u8;
        }
        ValueMap { table }
    }

    /// Builds a map from a full table. Returns `None` when an entry is above 127.
    pub fn from_table(table: [u8; 128]) -> Option<Self> {
        if table.iter().any(|&v| v > 127) {
            None
        } else {
            Some(ValueMap { table })
        }
    }

    /// A straight line from `in_lo` to `in_hi` onto `out_lo` to `out_hi`, with truncating integer steps.
    /// Inputs outside the input range are clamped to its ends. The ranges may run in either direction.
    /// Returns `None` when a bound is above 127 or the input range is empty.
    pub fn linear(in_lo: u8, in_hi: u8, out_lo: u8, out_hi: u8) -> Option<Self> {
        if in_lo > 127 || in_hi > 127 || out_lo > 127 || out_hi > 127 || in_lo == in_hi {
            return None;
        }
        let il = in_lo as i32;
        let ih = in_hi as i32;
        let ol = out_lo as i32;
        let oh = out_hi as i32;
        let mut table = [0u8; 128];
        for (i, slot) in table.iter_mut().enumerate() {
            let v = (i as i32).clamp(il.min(ih), il.max(ih));
            let out = ol + (v - il) * (oh - ol) / (ih - il);
            *slot = out as u8;
        }
        Some(ValueMap { table })
    }

    /// Sets one entry. Returns `false` and changes nothing when either value is above 127.
    pub fn set(&mut self, input: u8, output: u8) -> bool {
        if input > 127 || output > 127 {
            return false;
        }
        self.table[input as usize] = output;
        true
    }

    /// The mapped value. Only the low seven bits of `value` are used.
    pub fn map(&self, value: u8) -> u8 {
        self.table[(value & 0x7F) as usize]
    }
}

/// Rewrites one control change number to another, with its value passed through a map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlTransform {
    /// The control number that this transform matches.
    pub from: u8,
    /// The control number the event is sent as.
    pub to: u8,
    /// The map applied to the control's value.
    pub value: ValueMap,
}

impl ControlTransform {
    /// Returns the new control number and value when `control` matches `from`, otherwise `None`.
    pub fn apply(&self, control: u8, value: u8) -> Option<(u8, u8)> {
        if control == self.from {
            Some((self.to, self.value.map(value)))
        } else {
            None
        }
    }
}

/// One event on a thru connection. `channel` is 0 to 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThruEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    ControlChange { channel: u8, control: u8, value: u8 },
}

impl ThruEvent {
    pub fn channel(&self) -> u8 {
        match *self {
            ThruEvent::NoteOn { channel, .. }
            | ThruEvent::NoteOff { channel, .. }
            | ThruEvent::ControlChange { channel, .. } => channel,
        }
    }
}

/// What a thru connection does to the events it receives.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ThruParams {
    /// Only events on this channel pass. `None` passes every channel.
    pub channel: Option<u8>,
    /// Applied to note numbers of note-on and note-off events.
    pub note: ValueMap,
    /// Applied to velocities of note-on and note-off events.
    pub velocity: ValueMap,
    /// The first transform whose `from` matches a control change is applied. Other controls pass unchanged.
    pub controls: Vec<ControlTransform>,
}

/// A play-through connection that transforms events according to its parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThruConnection {
    params: ThruParams,
    enabled: bool,
}

impl ThruConnection {
    /// A new connection that is enabled.
    pub fn new(params: ThruParams) -> Self {
        ThruConnection { params, enabled: true }
    }

    pub fn params(&self) -> &ThruParams {
        &self.params
    }

    pub fn set_params(&mut self, params: ThruParams) {
        self.params = params;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The event to forward, or `None` when the connection is disabled or the channel filter drops it.
    pub fn process(&self, event: ThruEvent) -> Option<ThruEvent> {
        if !self.enabled {
            return None;
        }
        if let Some(channel) = self.params.channel {
            if event.channel() != channel {
                return None;
            }
        }
        let p = &self.params;
        Some(match event {
            ThruEvent::NoteOn { channel, note, velocity } => ThruEvent::NoteOn {
                channel,
                note: p.note.map(note),
                velocity: p.velocity.map(velocity),
            },
            ThruEvent::NoteOff { channel, note, velocity } => ThruEvent::NoteOff {
                channel,
                note: p.note.map(note),
                velocity: p.velocity.map(velocity),
            },
            ThruEvent::ControlChange { channel, control, value } => {
                let (control, value) = p
                    .controls
                    .iter()
                    .find_map(|t| t.apply(control, value))
                    .unwrap_or((control, value));
                ThruEvent::ControlChange { channel, control, value }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_map_keeps_values_and_masks_the_top_bit() {
        let m = ValueMap::identity();
        assert_eq!(m.map(60), 60);
        assert_eq!(m.map(127), 127);
        // 0x80 has only its low seven bits used, so it reads entry 0.
        assert_eq!(m.map(0x80), 0);
    }

    #[test]
    fn linear_map_truncates_each_step() {
        // 0..127 onto 0..64: 64 * 64 / 127 = 4096 / 127 = 32 with the remainder dropped.
        let m = ValueMap::linear(0, 127, 0, 64).unwrap();
        assert_eq!(m.map(0), 0);
        assert_eq!(m.map(64), 32);
        assert_eq!(m.map(127), 64);
    }

    #[test]
    fn linear_map_clamps_below_the_input_range() {
        // Input 100..127 onto 0..127: 113 gives 13 * 127 / 27 = 1651 / 27 = 61.
        let m = ValueMap::linear(100, 127, 0, 127).unwrap();
        assert_eq!(m.map(10), 0);
        assert_eq!(m.map(113), 61);
        assert_eq!(m.map(127), 127);
    }

    #[test]
    fn descending_ranges_are_allowed_and_empty_ones_are_refused() {
        // Input 127 down to 0 onto output 0 up to 127: value 64 gives (64 - 127) * 127 / -127 = 63.
        let m = ValueMap::linear(127, 0, 0, 127).unwrap();
        assert_eq!(m.map(127), 0);
        assert_eq!(m.map(64), 63);
        assert!(ValueMap::linear(5, 5, 0, 127).is_none());
        assert!(ValueMap::linear(0, 128, 0, 127).is_none());
    }

    #[test]
    fn set_and_from_table_refuse_values_above_127() {
        let mut m = ValueMap::identity();
        assert!(m.set(60, 72));
        assert_eq!(m.map(60), 72);
        assert!(!m.set(60, 128));
        assert_eq!(m.map(60), 72);
        let mut table = [0u8; 128];
        table[3] = 200;
        assert!(ValueMap::from_table(table).is_none());
    }

    #[test]
    fn control_transform_renumbers_only_its_control() {
        // CC 7 (volume) becomes CC 11 (expression), with its value halved from 0..127 to 0..64.
        let t = ControlTransform {
            from: 7,
            to: 11,
            value: ValueMap::linear(0, 127, 0, 64).unwrap(),
        };
        assert_eq!(t.apply(7, 127), Some((11, 64)));
        assert_eq!(t.apply(10, 127), None);
    }

    fn params_with_note_map_and_channel(channel: Option<u8>) -> ThruParams {
        let mut note = ValueMap::identity();
        note.set(60, 72);
        ThruParams {
            channel,
            note,
            velocity: ValueMap::linear(0, 127, 0, 64).unwrap(),
            controls: vec![ControlTransform {
                from: 7,
                to: 11,
                value: ValueMap::linear(0, 127, 0, 64).unwrap(),
            }],
        }
    }

    #[test]
    fn note_on_is_mapped_and_channel_filter_drops_others() {
        let conn = ThruConnection::new(params_with_note_map_and_channel(Some(2)));
        // Velocity 100 through 0..127 onto 0..64 is 6400 / 127 = 50.
        let on = ThruEvent::NoteOn { channel: 2, note: 60, velocity: 100 };
        assert_eq!(
            conn.process(on),
            Some(ThruEvent::NoteOn { channel: 2, note: 72, velocity: 50 })
        );
        let other = ThruEvent::NoteOn { channel: 3, note: 60, velocity: 100 };
        assert_eq!(conn.process(other), None);
    }

    #[test]
    fn note_off_uses_the_note_map_and_control_change_is_transformed() {
        let conn = ThruConnection::new(params_with_note_map_and_channel(None));
        let off = ThruEvent::NoteOff { channel: 15, note: 60, velocity: 0 };
        assert_eq!(
            conn.process(off),
            Some(ThruEvent::NoteOff { channel: 15, note: 72, velocity: 0 })
        );
        let cc = ThruEvent::ControlChange { channel: 0, control: 7, value: 127 };
        assert_eq!(
            conn.process(cc),
            Some(ThruEvent::ControlChange { channel: 0, control: 11, value: 64 })
        );
        // A control with no transform passes unchanged.
        let other = ThruEvent::ControlChange { channel: 0, control: 10, value: 90 };
        assert_eq!(conn.process(other), Some(other));
    }

    #[test]
    fn disabled_connection_forwards_nothing_and_default_params_pass_through() {
        let mut conn = ThruConnection::new(ThruParams::default());
        let on = ThruEvent::NoteOn { channel: 9, note: 36, velocity: 127 };
        assert_eq!(conn.process(on), Some(on));
        conn.set_enabled(false);
        assert!(!conn.enabled());
        assert_eq!(conn.process(on), None);
    }
}
