//! Render passes and reduced-resolution material work, described and checked before anything is submitted to the GPU.
//!
//! A pass draws into one to eight colour attachments of one size. Each attachment says what happens to its contents when
//! the pass starts (keep, clear, or ignore) and when it ends (keep or discard). Pipelines are fixed: the presenter owns
//! the quad, glass and shadow pipelines, and an application chooses one of them by name. Applications do not create
//! pipeline state.
//!
//! Material blur runs on a reduced copy of its target. The blur radius picks the divisor (1, 2 or 4), and the blurred
//! copy is scaled back up with linear filtering. Small targets are blurred at full size, because a reduced copy of
//! them would lose their edges.

/// The most colour attachments one pass may write.
pub const MAX_COLOR_ATTACHMENTS: usize = 8;

/// What a pass does with an attachment's contents when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadAction {
    /// Keep what is already in the attachment.
    Load,
    /// Fill the attachment with the clear colour first.
    Clear,
    /// The old contents are not read, so the pass may overwrite them freely.
    DontCare,
}

/// What a pass does with an attachment's contents when it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreAction {
    /// Keep the result for later passes and for the presenter.
    Store,
    /// The result is only needed during the pass, so it may be discarded.
    DontCare,
}

/// One colour attachment of a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorAttachment {
    pub load: LoadAction,
    pub store: StoreAction,
}

/// The compositor pipelines an application may name. The presenter compiles and owns them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedPipeline {
    /// Textured rectangles, the basic compositing step.
    Quad,
    /// The frosted glass composite.
    Glass,
    /// Soft window shadows.
    Shadow,
}

impl FixedPipeline {
    /// Every fixed pipeline.
    pub const ALL: [FixedPipeline; 3] = [FixedPipeline::Quad, FixedPipeline::Glass, FixedPipeline::Shadow];

    /// The name the pipeline is known by in requests.
    pub fn name(self) -> &'static str {
        match self {
            FixedPipeline::Quad => "quad",
            FixedPipeline::Glass => "glass",
            FixedPipeline::Shadow => "shadow",
        }
    }

    /// The pipeline with this name, or None for any other name.
    pub fn from_name(name: &str) -> Option<FixedPipeline> {
        FixedPipeline::ALL.into_iter().find(|p| p.name() == name)
    }
}

/// Why a render pass was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassError {
    /// The pass has no colour attachment.
    NoAttachments,
    /// The pass has more colour attachments than `MAX_COLOR_ATTACHMENTS`.
    TooManyAttachments(usize),
    /// The target has a zero width or height.
    EmptySize,
}

impl std::fmt::Display for PassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassError::NoAttachments => write!(f, "a render pass needs at least one colour attachment"),
            PassError::TooManyAttachments(n) => write!(f, "a render pass takes at most {MAX_COLOR_ATTACHMENTS} colour attachments, not {n}"),
            PassError::EmptySize => write!(f, "a render pass target needs a width and a height above zero"),
        }
    }
}

impl std::error::Error for PassError {}

/// A checked render pass: its pipeline, its target size and its colour attachments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderPass {
    pub pipeline: FixedPipeline,
    pub width: u32,
    pub height: u32,
    pub colors: Vec<ColorAttachment>,
}

impl RenderPass {
    /// Checks and builds a pass. Every attachment shares the pass size.
    pub fn new(pipeline: FixedPipeline, width: u32, height: u32, colors: Vec<ColorAttachment>) -> Result<RenderPass, PassError> {
        if colors.is_empty() {
            return Err(PassError::NoAttachments);
        }
        if colors.len() > MAX_COLOR_ATTACHMENTS {
            return Err(PassError::TooManyAttachments(colors.len()));
        }
        if width == 0 || height == 0 {
            return Err(PassError::EmptySize);
        }
        Ok(RenderPass { pipeline, width, height, colors })
    }
}

/// The reduced copy a blur of one radius works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlurPlan {
    /// How much smaller than the target the working copy is: 1, 2 or 4.
    pub divisor: u32,
    /// Width of the working copy in pixels.
    pub width: u32,
    /// Height of the working copy in pixels.
    pub height: u32,
}

/// The divisor for a blur of `radius` points: small radii keep full resolution, larger radii work at half or quarter
/// size.
pub fn blur_divisor(radius: u32) -> u32 {
    match radius {
        0..=3 => 1,
        4..=7 => 2,
        _ => 4,
    }
}

/// Plans a blur of a `width` by `height` target with `radius` points. The working copy is the target divided by the
/// divisor and rounded up, so no column or row is lost. A target that is less than twice the divisor in either side is
/// blurred at full size.
pub fn blur_plan(width: u32, height: u32, radius: u32) -> BlurPlan {
    let divisor = blur_divisor(radius);
    if divisor == 1 || width < 2 * divisor || height < 2 * divisor {
        return BlurPlan { divisor: 1, width, height };
    }
    BlurPlan { divisor, width: width.div_ceil(divisor), height: height.div_ceil(divisor) }
}
