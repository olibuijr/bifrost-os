//! The generic view of an audio unit (`UnitView`): the controls a host shows for any unit that has no view of its own.
//! It reads a unit's parameter handle and turns each parameter into a row with the control that suits it (a slider
//! with a log taper for frequencies, a switch for a boolean, a pop-up for a named list, a read-out for an output),
//! the text to show for its value, and its group, and it turns a drag, a click or a key press into a parameter change.
//!
//! The view is a model: it draws nothing. An app maps `Row::control` to its kit widgets (`controls::values`), calls
//! `refresh` each frame (a unit's parameters also move by themselves: ramps, presets, automation), and sends user
//! input back through `set_position`, `toggle`, `select`, `nudge` and `reset`. Changes go through the unit's own
//! `ParamHandle`, so they are clamped by the unit, ramped as the unit asks, and heard on the next block.
//!
//! Presets are `PresetStore`s of the unit: `apply_preset` selects one and the rows follow on the next `refresh`.

use bifrost_audio_engine::units::{ParamHandle, ParamSpec, ParamUnit, PresetStore, UnitError};

/// The control a parameter is shown with.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A continuous value on a slider.
    Slider,
    /// On or off.
    Toggle,
    /// One of a named list; the strings are the items.
    Popup(Vec<String>),
    /// A value the unit reports and the user cannot change.
    Readout,
}

/// One parameter as the view shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u32,
    pub name: String,
    /// The slash-separated group path ("EQ/Band 1"); empty for the top level.
    pub group: String,
    pub control: Control,
    pub value: f32,
    /// Where the value sits on its slider, 0 to 1, on the taper the unit's kind of value calls for.
    pub position: f32,
    /// The value as text with its unit ("-6.0 dB", "1.00 kHz", "On").
    pub text: String,
}

/// How far a key press moves a slider, as a fraction of its travel.
pub const NUDGE_STEP: f32 = 0.01;

/// Where a value sits on its slider. Frequencies use a log taper (equal distances are equal ratios), everything else
/// is linear.
pub fn position_of(spec: &ParamSpec, value: f32) -> f32 {
    let v = spec.clamp(value);
    if spec.unit == ParamUnit::Hertz && spec.min > 0.0 {
        let (lo, hi) = (spec.min.ln(), spec.max.ln());
        return ((v.ln() - lo) / (hi - lo)).clamp(0.0, 1.0);
    }
    spec.normalized(v)
}

/// The value at a slider position (the inverse of `position_of`). A position that is not a number counts as 0.
pub fn value_at(spec: &ParamSpec, position: f32) -> f32 {
    let p = if position.is_nan() { 0.0 } else { position.clamp(0.0, 1.0) };
    if spec.unit == ParamUnit::Hertz && spec.min > 0.0 {
        let (lo, hi) = (spec.min.ln(), spec.max.ln());
        return spec.clamp((lo + p * (hi - lo)).exp());
    }
    spec.value_from_normalized(p)
}

/// The text for a value with its unit.
pub fn format_value(spec: &ParamSpec, value: f32) -> String {
    let v = spec.clamp(value);
    match spec.unit {
        ParamUnit::Decibels => format!("{v:.1} dB"),
        ParamUnit::Hertz if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
        ParamUnit::Hertz => format!("{v:.0} Hz"),
        ParamUnit::Percent => format!("{v:.0} %"),
        ParamUnit::Seconds if v < 1.0 => format!("{:.0} ms", v * 1000.0),
        ParamUnit::Seconds => format!("{v:.2} s"),
        ParamUnit::Ratio => format!("{v:.2}"),
        ParamUnit::Boolean => (if v >= 0.5 { "On" } else { "Off" }).to_string(),
        ParamUnit::Indexed => {
            let i = (v - spec.min).round().max(0.0) as usize;
            spec.value_strings.get(i).cloned().unwrap_or_else(|| format!("{}", v.round() as i64))
        }
        ParamUnit::Generic => format!("{v:.2}"),
    }
}

fn control_for(spec: &ParamSpec) -> Control {
    if spec.read_only {
        Control::Readout
    } else if spec.unit == ParamUnit::Boolean {
        Control::Toggle
    } else if spec.unit == ParamUnit::Indexed && !spec.value_strings.is_empty() {
        Control::Popup(spec.value_strings.clone())
    } else {
        Control::Slider
    }
}

/// The generic view of one unit.
pub struct UnitView {
    handle: ParamHandle,
    rows: Vec<Row>,
}

impl UnitView {
    pub fn new(handle: ParamHandle) -> UnitView {
        let mut v = UnitView { handle, rows: Vec::new() };
        v.rows = v
            .handle
            .specs()
            .iter()
            .map(|s| Row {
                id: s.id,
                name: s.name.clone(),
                group: s.group.clone(),
                control: control_for(s),
                value: s.default,
                position: position_of(s, s.default),
                text: format_value(s, s.default),
            })
            .collect();
        v.refresh();
        v
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, id: u32) -> Option<&Row> {
        self.rows.get(id as usize)
    }

    /// The groups in the order they first appear, each with its rows: the sections of the view.
    pub fn sections(&self) -> Vec<(String, Vec<&Row>)> {
        let mut out: Vec<(String, Vec<&Row>)> = Vec::new();
        for r in &self.rows {
            match out.iter_mut().find(|(g, _)| *g == r.group) {
                Some((_, rows)) => rows.push(r),
                None => out.push((r.group.clone(), vec![r])),
            }
        }
        out
    }

    /// Reads every value again; returns how many rows changed (so the caller redraws only then).
    pub fn refresh(&mut self) -> usize {
        let mut changed = 0;
        for r in self.rows.iter_mut() {
            let Some(spec) = self.handle.spec(r.id) else { continue };
            let Ok(v) = self.handle.get(r.id) else { continue };
            if v != r.value || r.text.is_empty() {
                r.value = v;
                r.position = position_of(spec, v);
                r.text = format_value(spec, v);
                changed += 1;
            }
        }
        changed
    }

    fn spec(&self, id: u32) -> Result<&ParamSpec, UnitError> {
        self.handle.spec(id).ok_or(UnitError::BadParameter)
    }

    /// Sets a value (clamped, ramped as the unit asks) and updates the row. Returns the value that was set.
    pub fn set_value(&mut self, id: u32, value: f32) -> Result<f32, UnitError> {
        let set = self.handle.set(id, value)?;
        self.refresh();
        Ok(set)
    }

    /// A slider drag: the value for `position` on the taper.
    pub fn set_position(&mut self, id: u32, position: f32) -> Result<f32, UnitError> {
        let v = value_at(self.spec(id)?, position);
        self.set_value(id, v)
    }

    /// Flips a boolean parameter.
    pub fn toggle(&mut self, id: u32) -> Result<f32, UnitError> {
        let spec = self.spec(id)?;
        if spec.unit != ParamUnit::Boolean {
            return Err(UnitError::BadValue);
        }
        let on = self.handle.get(id)? >= 0.5;
        self.set_value(id, if on { 0.0 } else { 1.0 })
    }

    /// Picks item `index` of a pop-up parameter.
    pub fn select(&mut self, id: u32, index: usize) -> Result<f32, UnitError> {
        let spec = self.spec(id)?;
        if spec.unit != ParamUnit::Indexed || index >= spec.value_strings.len() {
            return Err(UnitError::BadValue);
        }
        let v = spec.min + index as f32;
        self.set_value(id, v)
    }

    /// A key press: moves a slider by `steps` of `NUDGE_STEP` of its travel, a pop-up by `steps` items, a switch
    /// to the side the sign says.
    pub fn nudge(&mut self, id: u32, steps: i32) -> Result<f32, UnitError> {
        let spec = self.spec(id)?.clone();
        let current = self.handle.get(id)?;
        match control_for(&spec) {
            Control::Readout => Err(UnitError::ReadOnly),
            Control::Toggle => self.set_value(id, if steps > 0 { 1.0 } else { 0.0 }),
            Control::Popup(items) => {
                let at = (current - spec.min).round() as i64;
                let to = (at + steps as i64).clamp(0, items.len() as i64 - 1);
                self.set_value(id, spec.min + to as f32)
            }
            Control::Slider => {
                let p = position_of(&spec, current) + steps as f32 * NUDGE_STEP;
                self.set_position(id, p)
            }
        }
    }

    /// Returns a parameter to its default.
    pub fn reset(&mut self, id: u32) -> Result<f32, UnitError> {
        let d = self.spec(id)?.default;
        self.set_value(id, d)
    }

    /// Selects a preset of the unit; the rows follow on the next `refresh` (this calls it).
    pub fn apply_preset(&mut self, presets: &mut PresetStore, number: i32) -> Result<(), UnitError> {
        presets.select(number, &self.handle)?;
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_audio_engine::units::{ParameterTree, Preset};

    fn tree() -> ParameterTree {
        ParameterTree::new(vec![
            ParamSpec::new(0, "Frequency", ParamUnit::Hertz, 20.0, 20_000.0, 1_000.0).unwrap().in_group("Filter"),
            ParamSpec::new(1, "Gain", ParamUnit::Decibels, -24.0, 24.0, 0.0).unwrap().in_group("Filter"),
            ParamSpec::new(2, "Bypass", ParamUnit::Boolean, 0.0, 1.0, 0.0).unwrap(),
            ParamSpec::new(3, "Type", ParamUnit::Indexed, 0.0, 2.0, 2.0).unwrap().with_value_strings(&["Low Pass", "High Pass", "Peaking"]),
            ParamSpec::new(4, "Level", ParamUnit::Generic, -10.0, 10.0, 0.0).unwrap().read_only(),
            ParamSpec::new(5, "Time", ParamUnit::Seconds, 0.0, 5.0, 0.25).unwrap().in_group("Delay"),
            ParamSpec::new(6, "Mix", ParamUnit::Percent, 0.0, 100.0, 50.0).unwrap().in_group("Delay"),
        ])
        .unwrap()
    }

    fn view() -> (ParameterTree, UnitView) {
        let t = tree();
        let v = UnitView::new(t.handle());
        (t, v)
    }

    #[test]
    fn each_parameter_gets_the_control_that_suits_it_and_its_text() {
        let (_t, v) = view();
        let controls: Vec<&Control> = v.rows().iter().map(|r| &r.control).collect();
        assert_eq!(controls[0], &Control::Slider);
        assert_eq!(controls[2], &Control::Toggle);
        assert_eq!(controls[3], &Control::Popup(vec!["Low Pass".into(), "High Pass".into(), "Peaking".into()]));
        assert_eq!(controls[4], &Control::Readout);
        let text: Vec<&str> = v.rows().iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, vec!["1.00 kHz", "0.0 dB", "Off", "Peaking", "0.00", "250 ms", "50 %"]);
    }

    #[test]
    fn frequencies_sit_on_a_log_taper_and_round_trip() {
        let (_t, v) = view();
        let spec = v.handle.spec(0).unwrap().clone();
        // 20 Hz to 20 kHz is three decades: 632 Hz is the geometric middle
        assert!((position_of(&spec, 632.455) - 0.5).abs() < 1e-3);
        assert_eq!(position_of(&spec, 20.0), 0.0);
        assert_eq!(position_of(&spec, 20_000.0), 1.0);
        for p in [0.0f32, 0.1, 0.5, 0.9, 1.0] {
            assert!((position_of(&spec, value_at(&spec, p)) - p).abs() < 1e-4, "{p}");
        }
        // linear for the rest
        let gain = v.handle.spec(1).unwrap().clone();
        assert_eq!(position_of(&gain, 0.0), 0.5);
        assert_eq!(value_at(&gain, 0.75), 12.0);
        assert_eq!(value_at(&gain, f32::NAN), -24.0);
    }

    #[test]
    fn dragging_a_slider_sets_the_unit_through_its_handle() {
        let (t, mut v) = view();
        let set = v.set_position(1, 1.0).unwrap();
        assert_eq!(set, 24.0);
        assert_eq!(t.handle().get(1).unwrap(), 24.0);
        assert_eq!(v.row(1).unwrap().text, "24.0 dB");
        assert_eq!(v.row(1).unwrap().position, 1.0);
        v.set_value(0, 5000.0).unwrap();
        assert_eq!(v.row(0).unwrap().text, "5.00 kHz");
        // out of range values are clamped by the unit
        assert_eq!(v.set_value(1, 100.0).unwrap(), 24.0);
    }

    #[test]
    fn toggle_select_nudge_and_reset() {
        let (t, mut v) = view();
        assert_eq!(v.toggle(2), Ok(1.0));
        assert_eq!(v.row(2).unwrap().text, "On");
        assert_eq!(v.toggle(2), Ok(0.0));
        assert_eq!(v.toggle(1), Err(UnitError::BadValue));
        assert_eq!(v.select(3, 1), Ok(1.0));
        assert_eq!(v.row(3).unwrap().text, "High Pass");
        assert_eq!(v.select(3, 7), Err(UnitError::BadValue));
        // arrow keys: a pop-up steps by item and stops at the ends, a slider by a hundredth of its travel
        assert_eq!(v.nudge(3, 5), Ok(2.0));
        assert_eq!(v.nudge(3, -9), Ok(0.0));
        let before = v.row(1).unwrap().position;
        v.nudge(1, 10).unwrap();
        assert!((v.row(1).unwrap().position - (before + 0.1)).abs() < 1e-5);
        assert_eq!(v.nudge(4, 1), Err(UnitError::ReadOnly));
        assert_eq!(v.nudge(2, 1), Ok(1.0));
        assert_eq!(v.reset(1), Ok(0.0));
        assert_eq!(t.handle().get(1).unwrap(), 0.0);
        assert_eq!(v.set_value(99, 1.0), Err(UnitError::BadParameter));
    }

    #[test]
    fn refresh_follows_changes_made_elsewhere_and_counts_them() {
        let (t, mut v) = view();
        assert_eq!(v.refresh(), 0);
        t.handle().set(1, -6.0).unwrap();
        t.handle().set(5, 2.0).unwrap();
        assert_eq!(v.refresh(), 2);
        assert_eq!(v.row(1).unwrap().text, "-6.0 dB");
        assert_eq!(v.row(5).unwrap().text, "2.00 s");
        assert_eq!(v.refresh(), 0);
    }

    #[test]
    fn rows_group_into_sections_in_first_seen_order() {
        let (_t, v) = view();
        let s = v.sections();
        let names: Vec<&str> = s.iter().map(|(g, _)| g.as_str()).collect();
        assert_eq!(names, vec!["Filter", "", "Delay"]);
        assert_eq!(s[0].1.len(), 2);
        assert_eq!(s[1].1.len(), 3);
        assert_eq!(s[2].1.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Time", "Mix"]);
    }

    #[test]
    fn presets_move_the_rows() {
        let (_t, mut v) = view();
        let mut store = PresetStore::new(vec![Preset { number: 0, name: "Dark".into(), values: vec![(0, 200.0), (1, -3.0), (2, 1.0)] }]).unwrap();
        v.apply_preset(&mut store, 0).unwrap();
        assert_eq!(v.row(0).unwrap().text, "200 Hz");
        assert_eq!(v.row(1).unwrap().text, "-3.0 dB");
        assert_eq!(v.row(2).unwrap().text, "On");
        assert_eq!(v.apply_preset(&mut store, 9), Err(UnitError::NoPreset));
    }

    #[test]
    fn indexed_values_without_a_name_show_their_number() {
        let spec = ParamSpec::new(0, "Voices", ParamUnit::Indexed, 1.0, 8.0, 1.0).unwrap();
        assert_eq!(format_value(&spec, 4.0), "4");
        let named = ParamSpec::new(0, "Mode", ParamUnit::Indexed, 0.0, 1.0, 0.0).unwrap().with_value_strings(&["A", "B"]);
        assert_eq!(format_value(&named, 1.0), "B");
    }
}
