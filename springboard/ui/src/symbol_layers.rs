//! Symbol layers: the pure rules a symbol-based image needs beyond its effects. Variable colour cycles the opacity of
//! each layer over a cycle (cumulative or iterative fill, dimmed or hidden inactive layers). A weight and a scale set
//! the stroke and size factors. A rendering mode gives each layer an opacity (hierarchical) or a palette slot.
//! The renderer applies the numbers; nothing here draws. The dim level, the weight steps, the scale factors and the
//! hierarchy opacities are BifrOSt's own values, not the reference system's tuning.

/// How a variable colour cycle fills the layers (`cumulative` and `iterative` fill styles).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// Each layer stays lit from its turn until the cycle ends.
    Cumulative,
    /// Only the layer whose turn it is is lit; the others return to rest.
    Iterative,
}

/// How inactive layers look while a cycle runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inactive {
    Dim,
    Hide,
}

/// Whether a cycle plays once through or reverses on each repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    NonReversing,
    Reversing,
}

/// Opacity of an inactive layer under `Inactive::Dim`.
pub const DIM_OPACITY: f32 = 0.3;

/// The progress through one cycle, from 0 to 1, after `elapsed` seconds of a cycle `period` seconds long. Negative
/// or NaN times read as the start; a period that is not above zero reads as the start too.
pub fn cycle_progress(elapsed: f32, period: f32, playback: Playback) -> f32 {
    if !(elapsed.is_finite() && period.is_finite() && period > 0.0) {
        return 0.0;
    }
    let phase = (elapsed.max(0.0) / period).fract();
    match playback {
        Playback::NonReversing => phase,
        Playback::Reversing => {
            if phase < 0.5 {
                phase * 2.0
            } else {
                (1.0 - phase) * 2.0
            }
        }
    }
}

/// Opacity of layer `layer` (0-based, of `layers`) at `progress` through the cycle. A layer past the count, or an
/// empty image, has opacity 0. Progress is clamped to 0..=1; NaN reads as 0.
///
/// Cumulative: layer `i` lights from `i / n` to full over one step, so at progress `p` it is `clamp(p * n - i, 0, 1)` lit.
/// Iterative: the layer `floor(p * n)` (capped at the last layer) is lit fully; the others are not lit.
/// An unlit layer rests at `DIM_OPACITY` when dimmed, or at 0 when hidden.
pub fn layer_opacity(layer: usize, layers: usize, progress: f32, fill: Fill, inactive: Inactive) -> f32 {
    if layer >= layers {
        return 0.0;
    }
    let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 };
    let lit = match fill {
        Fill::Cumulative => (p * layers as f32 - layer as f32).clamp(0.0, 1.0),
        Fill::Iterative => {
            let active = ((p * layers as f32).floor() as usize).min(layers - 1);
            if layer == active {
                1.0
            } else {
                0.0
            }
        }
    };
    let rest = match inactive {
        Inactive::Dim => DIM_OPACITY,
        Inactive::Hide => 0.0,
    };
    rest + (1.0 - rest) * lit
}

/// The weight steps of a symbol, lightest to heaviest. `Regular` is the baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    UltraLight,
    Thin,
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
    Heavy,
    Black,
}

impl Weight {
    /// Position from the lightest step (0) to the heaviest (8).
    pub fn index(self) -> usize {
        self as usize
    }

    /// Multiplier on the baseline stroke width: 1.0 at `Regular`, plus or minus 0.15 for each step away from it.
    pub fn stroke_factor(self) -> f32 {
        1.0 + 0.15 * (self.index() as f32 - 3.0)
    }
}

/// The size steps of a symbol relative to the text beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Small,
    Medium,
    Large,
}

impl Scale {
    /// Multiplier on the symbol's base size.
    pub fn factor(self) -> f32 {
        match self {
            Scale::Small => 0.75,
            Scale::Medium => 1.0,
            Scale::Large => 1.25,
        }
    }
}

/// How a symbol's layers take colour (`SymbolRenderingMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderingMode {
    /// One colour for every layer.
    Monochrome,
    /// One colour, with layers stepped down in opacity by their rank.
    Hierarchical,
    /// A list of colours; layer `n` takes slot `n`, and the last slot repeats.
    Palette,
    /// Each layer keeps its own fixed colour.
    Multicolor,
}

/// Opacity steps for hierarchical layers, by rank: the primary layer is full, then half, then 0.3.
pub const HIERARCHY: [f32; 3] = [1.0, 0.5, 0.3];

/// Opacity of a layer of the given rank (0 is the primary layer) under a rendering mode. Ranks past the last
/// hierarchy step use the last step.
pub fn rendering_opacity(mode: RenderingMode, rank: usize) -> f32 {
    match mode {
        RenderingMode::Hierarchical => HIERARCHY[rank.min(HIERARCHY.len() - 1)],
        _ => 1.0,
    }
}

/// The palette slot a layer of the given rank takes, or `None` when the palette is empty. The last slot repeats.
pub fn palette_slot(rank: usize, palette_len: usize) -> Option<usize> {
    if palette_len == 0 {
        None
    } else {
        Some(rank.min(palette_len - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn cumulative_dimmed_layers_light_in_turn_over_four_layers() {
        // n = 4, progress 0.5 gives p * n = 2: layers 0 and 1 are lit, 2 and 3 rest at the dim level.
        let at = |i| layer_opacity(i, 4, 0.5, Fill::Cumulative, Inactive::Dim);
        assert!(close(at(0), 1.0));
        assert!(close(at(1), 1.0));
        assert!(close(at(2), DIM_OPACITY));
        assert!(close(at(3), DIM_OPACITY));
        // progress 0.375 gives p * n = 1.5: layer 1 is half lit, 0.3 + 0.7 * 0.5 = 0.65.
        assert!(close(layer_opacity(1, 4, 0.375, Fill::Cumulative, Inactive::Dim), 0.65));
    }

    #[test]
    fn iterative_hidden_layers_show_only_the_active_layer() {
        // progress 0.5 with n = 4 makes layer floor(2.0) = 2 active; hidden layers rest at 0.
        let at = |i| layer_opacity(i, 4, 0.5, Fill::Iterative, Inactive::Hide);
        assert!(close(at(0), 0.0));
        assert!(close(at(1), 0.0));
        assert!(close(at(2), 1.0));
        assert!(close(at(3), 0.0));
        // progress 1.0 would be index 4; it is capped at the last layer.
        assert!(close(layer_opacity(3, 4, 1.0, Fill::Iterative, Inactive::Hide), 1.0));
    }

    #[test]
    fn layers_past_the_count_and_empty_images_have_no_opacity() {
        assert!(close(layer_opacity(4, 4, 0.5, Fill::Cumulative, Inactive::Dim), 0.0));
        assert!(close(layer_opacity(0, 0, 0.5, Fill::Iterative, Inactive::Dim), 0.0));
        // NaN progress reads as the start: nothing past layer 0 is lit.
        assert!(close(layer_opacity(1, 3, f32::NAN, Fill::Cumulative, Inactive::Hide), 0.0));
    }

    #[test]
    fn cycle_progress_reverses_on_the_way_back() {
        // period 2: elapsed 1 is phase 0.5. Non-reversing ends the cycle at 0.5; reversing peaks at 1.0.
        assert!(close(cycle_progress(1.0, 2.0, Playback::NonReversing), 0.5));
        assert!(close(cycle_progress(1.0, 2.0, Playback::Reversing), 1.0));
        // elapsed 0.5 is phase 0.25; reversing is on the way up, 0.5.
        assert!(close(cycle_progress(0.5, 2.0, Playback::Reversing), 0.5));
        // elapsed 3.0 is one full cycle plus phase 0.5 again.
        assert!(close(cycle_progress(3.0, 2.0, Playback::NonReversing), 0.5));
    }

    #[test]
    fn cycle_progress_treats_bad_times_as_the_start() {
        assert!(close(cycle_progress(-4.0, 2.0, Playback::NonReversing), 0.0));
        assert!(close(cycle_progress(1.0, 0.0, Playback::NonReversing), 0.0));
        assert!(close(cycle_progress(f32::NAN, 2.0, Playback::Reversing), 0.0));
    }

    #[test]
    fn weight_factors_step_by_0_15_from_regular() {
        assert!(close(Weight::Regular.stroke_factor(), 1.0));
        assert!(close(Weight::UltraLight.stroke_factor(), 0.55));
        assert!(close(Weight::Semibold.stroke_factor(), 1.3));
        assert!(close(Weight::Black.stroke_factor(), 1.75));
        assert_eq!(Weight::Black.index(), 8);
        assert_eq!(Weight::Regular.index(), 3);
    }

    #[test]
    fn scale_factors_are_fixed_steps() {
        assert!(close(Scale::Small.factor(), 0.75));
        assert!(close(Scale::Medium.factor(), 1.0));
        assert!(close(Scale::Large.factor(), 1.25));
    }

    #[test]
    fn hierarchical_layers_step_down_and_the_last_step_repeats() {
        assert!(close(rendering_opacity(RenderingMode::Hierarchical, 0), 1.0));
        assert!(close(rendering_opacity(RenderingMode::Hierarchical, 1), 0.5));
        assert!(close(rendering_opacity(RenderingMode::Hierarchical, 2), 0.3));
        assert!(close(rendering_opacity(RenderingMode::Hierarchical, 7), 0.3));
        // The other modes do not step opacity by rank.
        assert!(close(rendering_opacity(RenderingMode::Monochrome, 2), 1.0));
        assert!(close(rendering_opacity(RenderingMode::Multicolor, 2), 1.0));
    }

    #[test]
    fn palette_slots_repeat_the_last_colour_and_empty_palettes_have_none() {
        assert_eq!(palette_slot(0, 3), Some(0));
        assert_eq!(palette_slot(2, 3), Some(2));
        assert_eq!(palette_slot(5, 3), Some(2));
        assert_eq!(palette_slot(0, 0), None);
    }
}
