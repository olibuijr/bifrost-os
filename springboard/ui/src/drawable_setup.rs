//! Drawable setup for a GPU render view: the size, opacity and device an app asks for, checked against the
//! largest side the device accepts, and the four-character format code the presenter allocates buffers with.
//! The device itself and the buffers stay with the shell; this model decides whether a request is valid.

/// Four-character code of an opaque 8-bit colour buffer, the format the presenter uses for opaque surfaces.
pub const FORMAT_OPAQUE: u32 = u32::from_le_bytes(*b"XB24");
/// Four-character code of an 8-bit colour buffer with an alpha channel.
pub const FORMAT_ALPHA: u32 = u32::from_le_bytes(*b"AB24");

/// Bytes per pixel of the 8-bit formats above.
pub const BYTES_PER_PIXEL: u64 = 4;

/// The device a drawable is made on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum DevicePreference {
    /// The device the shell presents on.
    #[default]
    Default,
    /// A DRM render node, such as `/dev/dri/renderD128`.
    RenderNode(String),
}

/// What a view asks for before any buffer exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawableSetup {
    pub width: u32,
    pub height: u32,
    /// True when the buffer keeps an alpha channel.
    pub alpha: bool,
    pub device: DevicePreference,
}

/// A request the shell can allocate: a size, a format code and the device it lands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawable {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub device: DevicePreference,
}

/// Why a drawable request is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// Width or height is zero, so there is nothing to draw into.
    ZeroSize { width: u32, height: u32 },
    /// The longer side exceeds the largest side the device accepts.
    TooLarge { side: u32, max: u32 },
    /// The render node is not of the form `/dev/dri/renderD<number>`.
    BadRenderNode(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::ZeroSize { width, height } => write!(f, "a drawable needs a size above zero, got {width}x{height}"),
            SetupError::TooLarge { side, max } => write!(f, "a drawable side of {side} exceeds the device limit of {max}"),
            SetupError::BadRenderNode(path) => write!(f, "{path} is not a render node path"),
        }
    }
}

impl std::error::Error for SetupError {}

/// True for `/dev/dri/renderD<digits>`.
fn is_render_node(path: &str) -> bool {
    path.strip_prefix("/dev/dri/renderD").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

impl DrawableSetup {
    /// Checks the request against the device's largest accepted side and returns the drawable the shell allocates.
    pub fn validate(&self, max_side: u32) -> Result<Drawable, SetupError> {
        if self.width == 0 || self.height == 0 {
            return Err(SetupError::ZeroSize { width: self.width, height: self.height });
        }
        let side = self.width.max(self.height);
        if side > max_side {
            return Err(SetupError::TooLarge { side, max: max_side });
        }
        if let DevicePreference::RenderNode(path) = &self.device {
            if !is_render_node(path) {
                return Err(SetupError::BadRenderNode(path.clone()));
            }
        }
        let format = if self.alpha { FORMAT_ALPHA } else { FORMAT_OPAQUE };
        Ok(Drawable { width: self.width, height: self.height, format, device: self.device.clone() })
    }
}

impl Drawable {
    /// Bytes one row of pixels occupies in the buffer, without row padding.
    pub fn row_bytes(&self) -> u64 {
        u64::from(self.width) * BYTES_PER_PIXEL
    }

    /// Bytes the whole buffer occupies, without row padding.
    pub fn byte_len(&self) -> u64 {
        self.row_bytes() * u64::from(self.height)
    }
}
