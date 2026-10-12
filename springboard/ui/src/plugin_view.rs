//! Plugin view model: the configuration a host gives a plugin's user interface (size, and whether the host draws
//! its own controls), the generic view built from a parameter tree (one control per writable parameter, in the
//! groups the tree gives), and the panner view's pan position with its gains. These are pure models. No drawing
//! happens here. The pan law is equal power, a common choice that the official pages do not specify.

use crate::plugin_host::{collect_parameters, HostError, Node, ParameterSpec, ParameterTree, Unit};

/// How a host presents a plugin's view: its size in points, and whether the host draws its own controls too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewConfiguration {
    pub width: f64,
    pub height: f64,
    pub host_has_controller: bool,
}

impl ViewConfiguration {
    /// A checked configuration. Width and height must be finite and above zero.
    pub fn new(width: f64, height: f64, host_has_controller: bool) -> Result<Self, HostError> {
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            Ok(ViewConfiguration { width, height, host_has_controller })
        } else {
            Err(HostError::InvalidValue)
        }
    }
}

/// One control of the generic view.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A continuous value. `position` is 0 at the minimum and 1 at the maximum.
    Slider { address: u64, name: String, position: f32, label: String },
    /// A boolean value. The value is on from 0.5 up.
    Toggle { address: u64, name: String, on: bool },
    /// An indexed value shown as one of its names.
    Choice { address: u64, name: String, index: usize, options: Vec<String> },
}

/// A group of controls. Top-level parameters that sit side by side share one section with an empty name.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub identifier: String,
    pub name: String,
    pub controls: Vec<Control>,
}

/// The generic view of a parameter tree. Read-only parameters get no control.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericView {
    sections: Vec<Section>,
}

impl GenericView {
    /// The view for the tree as it stands now, so each control shows the current value.
    pub fn new(tree: &ParameterTree) -> Self {
        let mut sections: Vec<Section> = Vec::new();
        for node in tree.roots() {
            let (identifier, name) = match node {
                Node::Parameter(_) => (String::new(), String::new()),
                Node::Group { identifier, name, .. } => (identifier.clone(), name.clone()),
            };
            let mut found: Vec<&ParameterSpec> = Vec::new();
            collect_parameters(std::slice::from_ref(node), &mut found);
            let controls: Vec<Control> = found
                .into_iter()
                .filter(|spec| spec.writable)
                .map(|spec| control_for(spec, tree.value(spec.address).unwrap_or(spec.default)))
                .collect();
            if controls.is_empty() {
                continue;
            }
            if identifier.is_empty() {
                if let Some(last) = sections.last_mut() {
                    if last.identifier.is_empty() {
                        last.controls.extend(controls);
                        continue;
                    }
                }
            }
            sections.push(Section { identifier, name, controls });
        }
        GenericView { sections }
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The control for an address, if the view has one.
    pub fn control(&self, address: u64) -> Option<&Control> {
        self.sections
            .iter()
            .flat_map(|s| s.controls.iter())
            .find(|c| control_address(c) == address)
    }
}

/// The value a slider position maps to, for a parameter in the tree. `None` when the address is unknown.
pub fn value_for_position(tree: &ParameterTree, address: u64, position: f32) -> Option<f32> {
    tree.parameter(address).map(|spec| spec.value_from_normalized(position))
}

fn control_address(control: &Control) -> u64 {
    match control {
        Control::Slider { address, .. } | Control::Toggle { address, .. } | Control::Choice { address, .. } => *address,
    }
}

fn control_for(spec: &ParameterSpec, value: f32) -> Control {
    let address = spec.address;
    let name = spec.name.clone();
    match spec.unit {
        Unit::Boolean => Control::Toggle { address, name, on: value >= 0.5 },
        Unit::Indexed if !spec.value_strings.is_empty() => {
            let last = spec.value_strings.len() - 1;
            let index = (value.round().max(0.0) as usize).min(last);
            Control::Choice { address, name, index, options: spec.value_strings.clone() }
        }
        _ => Control::Slider { address, name, position: spec.normalized(value), label: label(spec.unit, value) },
    }
}

fn label(unit: Unit, value: f32) -> String {
    let suffix = match unit {
        Unit::Percent => "%",
        Unit::Seconds => " s",
        Unit::Hertz => " Hz",
        Unit::Decibels => " dB",
        Unit::Generic | Unit::Indexed | Unit::Boolean | Unit::Pan => "",
    };
    format!("{:.2}{}", value, suffix)
}

/// The pan control of a panner view. The position runs from -1 (left) to 1 (right), and the gains follow an
/// equal-power law, so the summed power stays the same across the range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PannerView {
    address: u64,
    position: f32,
}

impl PannerView {
    /// A panner for the pan parameter at `address`, centred.
    pub fn new(address: u64) -> Self {
        PannerView { address, position: 0.0 }
    }

    pub fn address(&self) -> u64 {
        self.address
    }

    pub fn position(&self) -> f32 {
        self.position
    }

    /// Sets the position, clamped to -1 to 1. A position that is not finite is refused.
    pub fn set_position(&mut self, position: f32) -> Result<(), HostError> {
        if !position.is_finite() {
            return Err(HostError::InvalidValue);
        }
        self.position = position.clamp(-1.0, 1.0);
        Ok(())
    }

    /// Sets the position from a horizontal fraction of the panel: 0 is the left edge, 1 the right edge.
    pub fn set_from_fraction(&mut self, fraction: f32) -> Result<(), HostError> {
        if !fraction.is_finite() {
            return Err(HostError::InvalidValue);
        }
        self.position = fraction.clamp(0.0, 1.0) * 2.0 - 1.0;
        Ok(())
    }

    /// The left and right gains at the current position.
    pub fn gains(&self) -> (f32, f32) {
        let angle = (self.position + 1.0) * std::f32::consts::FRAC_PI_4;
        (angle.cos(), angle.sin())
    }

    /// Writes the position to the pan parameter of the tree.
    pub fn apply(&self, tree: &mut ParameterTree) -> Result<(), HostError> {
        tree.set_value(self.address, self.position).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_host::{Node, ParameterSpec, ParameterTree, Unit};

    fn cutoff() -> ParameterSpec {
        ParameterSpec::new(10, "cutoff", "Cutoff", 20.0, 20_000.0, 1_000.0, Unit::Hertz).unwrap()
    }

    fn bypass() -> ParameterSpec {
        ParameterSpec::new(11, "bypass", "Bypass", 0.0, 1.0, 0.0, Unit::Boolean).unwrap()
    }

    fn meter() -> ParameterSpec {
        ParameterSpec::new(12, "meter", "Meter", 0.0, 1.0, 0.0, Unit::Generic).unwrap().read_only()
    }

    fn mode() -> ParameterSpec {
        ParameterSpec::new(20, "mode", "Mode", 0.0, 2.0, 1.0, Unit::Indexed)
            .unwrap()
            .with_value_strings(&["Low", "Band", "High"])
    }

    fn sample_tree() -> ParameterTree {
        ParameterTree::new(vec![
            Node::Group {
                identifier: "filter".to_string(),
                name: "Filter".to_string(),
                children: vec![Node::Parameter(cutoff()), Node::Parameter(bypass()), Node::Parameter(meter())],
            },
            Node::Parameter(mode()),
        ])
        .unwrap()
    }

    #[test]
    fn view_configuration_needs_a_positive_finite_size() {
        assert_eq!(ViewConfiguration::new(0.0, 100.0, true), Err(HostError::InvalidValue));
        assert_eq!(ViewConfiguration::new(f64::NAN, 100.0, true), Err(HostError::InvalidValue));
        let cfg = ViewConfiguration::new(640.0, 480.0, false).unwrap();
        assert_eq!(cfg.width, 640.0);
        assert_eq!(cfg.height, 480.0);
        assert!(!cfg.host_has_controller);
    }

    #[test]
    fn generic_view_groups_parameters_and_skips_read_only_ones() {
        let view = GenericView::new(&sample_tree());
        let sections = view.sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].identifier, "filter");
        assert_eq!(sections[0].name, "Filter");
        assert_eq!(sections[0].controls.len(), 2);
        assert_eq!(sections[1].name, "");
        assert_eq!(sections[1].controls.len(), 1);
        assert!(view.control(12).is_none());

        match &sections[0].controls[0] {
            Control::Slider { address, label, position, .. } => {
                assert_eq!(*address, 10);
                assert_eq!(label, "1000.00 Hz");
                // (1000 - 20) / (20000 - 20) = 980 / 19980
                assert!((position - 0.049049).abs() < 0.0001);
            }
            other => panic!("expected a slider, got {:?}", other),
        }
        match &sections[0].controls[1] {
            Control::Toggle { on, .. } => assert!(!on),
            other => panic!("expected a toggle, got {:?}", other),
        }
    }

    #[test]
    fn indexed_values_show_their_name_and_clamp_to_the_last_option() {
        let mut tree = sample_tree();
        let view = GenericView::new(&tree);
        match view.control(20) {
            Some(Control::Choice { index, options, .. }) => {
                assert_eq!(*index, 1);
                assert_eq!(options.len(), 3);
            }
            other => panic!("expected a choice, got {:?}", other),
        }
        tree.set_value(20, 9.0).unwrap();
        let view = GenericView::new(&tree);
        match view.control(20) {
            Some(Control::Choice { index, .. }) => assert_eq!(*index, 2),
            other => panic!("expected a choice, got {:?}", other),
        }
    }

    #[test]
    fn slider_positions_map_back_to_values() {
        let tree = sample_tree();
        let value = value_for_position(&tree, 10, 0.5).unwrap();
        assert!((value - 10_010.0).abs() < 0.01);
        assert_eq!(value_for_position(&tree, 99, 0.5), None);
        // Positions outside 0 to 1 are clamped to the range.
        assert_eq!(value_for_position(&tree, 10, 2.0), Some(20_000.0));
    }

    #[test]
    fn panner_gains_follow_the_equal_power_law() {
        let mut panner = PannerView::new(5);
        let (l, r) = panner.gains();
        assert!((l - 0.70710678).abs() < 1e-5);
        assert!((r - 0.70710678).abs() < 1e-5);

        panner.set_position(-1.0).unwrap();
        let (l, r) = panner.gains();
        assert!((l - 1.0).abs() < 1e-5 && r.abs() < 1e-5);

        panner.set_position(1.0).unwrap();
        let (l, r) = panner.gains();
        assert!(l.abs() < 1e-5 && (r - 1.0).abs() < 1e-5);

        panner.set_position(0.3).unwrap();
        let (l, r) = panner.gains();
        assert!((l * l + r * r - 1.0).abs() < 1e-5);
    }

    #[test]
    fn panner_takes_fractions_clamps_and_writes_the_pan_parameter() {
        let mut panner = PannerView::new(30);
        panner.set_from_fraction(0.75).unwrap();
        assert!((panner.position() - 0.5).abs() < 1e-6);
        panner.set_from_fraction(2.0).unwrap();
        assert_eq!(panner.position(), 1.0);
        assert_eq!(panner.set_position(f32::NAN), Err(HostError::InvalidValue));
        assert_eq!(panner.set_from_fraction(f32::INFINITY), Err(HostError::InvalidValue));

        let pan = ParameterSpec::new(30, "pan", "Pan", -1.0, 1.0, 0.0, Unit::Pan).unwrap();
        let mut tree = ParameterTree::new(vec![Node::Parameter(pan)]).unwrap();
        panner.apply(&mut tree).unwrap();
        assert_eq!(tree.value(30), Some(1.0));
    }
}
