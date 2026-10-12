//! Generic parameter view: the model behind a control panel that lists every parameter of an audio processor with a
//! slider per parameter, and hides the expert parameters unless the display flags ask for them. Each parameter maps
//! between its value and a normalised position in 0 to 1, linearly or on a logarithmic scale. The view draws nothing;
//! a renderer lays out the rows that [`GenericView::visible`] returns.
//! Parameters with an unusable range (not finite, minimum not below maximum, or a logarithmic range starting at zero or
//! below) are dropped when the view is built, so every row has a defined mapping.

/// How a parameter's value maps to its slider position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// Position is proportional to the value.
    Linear,
    /// Position is proportional to the logarithm of the value. Needs a minimum above zero.
    Logarithmic,
}

/// One parameter of the processor.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: u32,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub value: f32,
    pub scale: Scale,
    /// An expert parameter is hidden unless [`DisplayFlags::shows_expert`] is set.
    pub expert: bool,
}

impl ParamSpec {
    /// A linear, non-expert parameter whose value starts at its default.
    pub fn linear(id: u32, name: &str, min: f32, max: f32, default: f32) -> Self {
        Self {
            id,
            name: name.to_string(),
            min,
            max,
            default,
            value: default,
            scale: Scale::Linear,
            expert: false,
        }
    }

    /// Whether the range and default describe a usable parameter.
    pub fn is_valid(&self) -> bool {
        if !(self.min.is_finite() && self.max.is_finite() && self.default.is_finite()) {
            return false;
        }
        if !(self.min < self.max) {
            return false;
        }
        match self.scale {
            Scale::Linear => true,
            Scale::Logarithmic => self.min > 0.0,
        }
    }

    /// The value clamped into the parameter range. A NaN becomes the default.
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_nan() {
            self.default
        } else {
            value.clamp(self.min, self.max)
        }
    }

    /// The slider position for the current value, in 0 to 1.
    pub fn normalized(&self) -> f32 {
        position_of(self.scale, self.min, self.max, self.value)
    }

    /// The value at a slider position. The position is clamped into 0 to 1.
    pub fn value_at(&self, position: f32) -> f32 {
        let t = if position.is_nan() { 0.0 } else { position.clamp(0.0, 1.0) };
        match self.scale {
            Scale::Linear => self.min + t * (self.max - self.min),
            Scale::Logarithmic => {
                let (lo, hi) = (self.min.ln(), self.max.ln());
                (lo + t * (hi - lo)).exp()
            }
        }
    }
}

fn position_of(scale: Scale, min: f32, max: f32, value: f32) -> f32 {
    let v = value.clamp(min, max);
    let t = match scale {
        Scale::Linear => (v - min) / (max - min),
        Scale::Logarithmic => (v.ln() - min.ln()) / (max.ln() - min.ln()),
    };
    t.clamp(0.0, 1.0)
}

/// Display options for a generic view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayFlags {
    /// Show the expert parameters as well as the ordinary ones.
    pub shows_expert: bool,
}

/// The rows of a generic parameter view, in the order the processor lists them.
#[derive(Clone, Debug, Default)]
pub struct GenericView {
    params: Vec<ParamSpec>,
    flags: DisplayFlags,
}

impl GenericView {
    /// Builds the view. Invalid parameters are dropped; the rest keep their order.
    pub fn new(params: Vec<ParamSpec>, flags: DisplayFlags) -> Self {
        let params = params.into_iter().filter(ParamSpec::is_valid).collect();
        Self { params, flags }
    }

    /// The display flags in effect.
    pub fn flags(&self) -> DisplayFlags {
        self.flags
    }

    /// Changes the display flags, for example when the user toggles expert parameters.
    pub fn set_flags(&mut self, flags: DisplayFlags) {
        self.flags = flags;
    }

    /// The parameters the view shows, in order. An expert parameter is included only when the flags show expert ones.
    pub fn visible(&self) -> Vec<&ParamSpec> {
        self.params
            .iter()
            .filter(|p| !p.expert || self.flags.shows_expert)
            .collect()
    }

    /// Sets a parameter's value, clamped into its range. Returns `false` when no parameter has that id.
    pub fn set_value(&mut self, id: u32, value: f32) -> bool {
        match self.params.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.value = p.clamp(value);
                true
            }
            None => false,
        }
    }

    /// Returns a parameter to its default value. Returns `false` when no parameter has that id.
    pub fn reset(&mut self, id: u32) -> bool {
        match self.params.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.value = p.default;
                true
            }
            None => false,
        }
    }

    /// The slider position of a parameter in 0 to 1, or `None` when the id is unknown.
    pub fn position(&self, id: u32) -> Option<f32> {
        self.params.iter().find(|p| p.id == id).map(ParamSpec::normalized)
    }

    /// Sets a parameter from a slider position in 0 to 1. Returns `false` when the id is unknown.
    pub fn set_position(&mut self, id: u32, position: f32) -> bool {
        match self.params.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.value = p.value_at(position);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain() -> ParamSpec {
        ParamSpec::linear(1, "Gain", 0.0, 10.0, 5.0)
    }

    fn cutoff() -> ParamSpec {
        ParamSpec {
            id: 2,
            name: "Cutoff".to_string(),
            min: 20.0,
            max: 20000.0,
            default: 1000.0,
            value: 1000.0,
            scale: Scale::Logarithmic,
            expert: false,
        }
    }

    fn expert_bias() -> ParamSpec {
        ParamSpec { id: 3, expert: true, ..ParamSpec::linear(3, "Bias", -1.0, 1.0, 0.0) }
    }

    #[test]
    fn expert_parameters_are_hidden_unless_the_flag_is_set() {
        let mut view = GenericView::new(vec![gain(), expert_bias()], DisplayFlags::default());
        let ids: Vec<u32> = view.visible().iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1]);

        view.set_flags(DisplayFlags { shows_expert: true });
        let ids: Vec<u32> = view.visible().iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[test]
    fn set_value_clamps_into_the_range() {
        let mut view = GenericView::new(vec![gain()], DisplayFlags::default());
        assert!(view.set_value(1, 12.0));
        assert_eq!(view.visible()[0].value, 10.0);
        assert!(view.set_value(1, -1.0));
        assert_eq!(view.visible()[0].value, 0.0);
        assert!(!view.set_value(99, 1.0));
    }

    #[test]
    fn linear_position_is_the_fraction_of_the_range() {
        let mut view = GenericView::new(vec![gain()], DisplayFlags::default());
        view.set_value(1, 2.5);
        assert!((view.position(1).unwrap() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn logarithmic_position_is_the_fraction_of_the_log_range() {
        // ln(200 / 20) / ln(20000 / 20) = ln(10) / ln(1000) = 1 / 3.
        let mut view = GenericView::new(vec![cutoff()], DisplayFlags::default());
        view.set_value(2, 200.0);
        assert!((view.position(2).unwrap() - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn half_way_on_a_logarithmic_slider_is_the_geometric_mean() {
        // The geometric mean of 20 and 20000 is sqrt(400000), about 632.456.
        let mut view = GenericView::new(vec![cutoff()], DisplayFlags::default());
        assert!(view.set_position(2, 0.5));
        let value = view.visible()[0].value;
        assert!((value - 632.455_5).abs() < 0.01, "got {value}");
    }

    #[test]
    fn invalid_ranges_are_dropped_and_reset_restores_the_default() {
        let bad_log = ParamSpec { scale: Scale::Logarithmic, ..ParamSpec::linear(4, "Bad", 0.0, 1.0, 0.5) };
        let flat = ParamSpec::linear(5, "Flat", 1.0, 1.0, 1.0);
        let mut view = GenericView::new(vec![gain(), bad_log, flat], DisplayFlags::default());
        assert_eq!(view.visible().len(), 1);

        view.set_value(1, 9.0);
        assert!(view.reset(1));
        assert_eq!(view.visible()[0].value, 5.0);
        assert!(!view.reset(4));
    }
}
