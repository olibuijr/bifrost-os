//! The drawable of a view: the colour, depth and stencil buffers behind it, the multisample count, and its size, as a pure
//! model. A view that draws with a graphics API asks for these formats before it draws; this module checks that the
//! combination is valid and counts the bytes the buffers take. It creates no GPU object.
//!
//! Rules: a drawable needs a colour buffer. The sample count is 1, 2, 4 or 8. With more than one sample, the colour and
//! depth buffers each hold every sample for every pixel, and the memory count multiplies them by the sample count.

/// The colour buffer format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorFormat {
    /// No colour buffer. Invalid for a drawable.
    None,
    /// 16 bits per pixel, 5 red, 6 green, 5 blue.
    Rgb565,
    /// 32 bits per pixel, 8 bits a channel.
    Rgba8,
    /// 32 bits per pixel, blue first, 8 bits a channel.
    Bgra8,
    /// 64 bits per pixel, four 16-bit float channels.
    Rgba16Float,
}

impl ColorFormat {
    /// Bytes one sample of this format takes.
    pub fn bytes_per_sample(self) -> u64 {
        match self {
            ColorFormat::None => 0,
            ColorFormat::Rgb565 => 2,
            ColorFormat::Rgba8 | ColorFormat::Bgra8 => 4,
            ColorFormat::Rgba16Float => 8,
        }
    }
}

/// The depth buffer format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthFormat {
    None,
    Depth16,
    /// Stored in 32 bits.
    Depth24,
    Depth32Float,
}

impl DepthFormat {
    pub fn bytes_per_sample(self) -> u64 {
        match self {
            DepthFormat::None => 0,
            DepthFormat::Depth16 => 2,
            DepthFormat::Depth24 | DepthFormat::Depth32Float => 4,
        }
    }
}

/// The stencil buffer format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StencilFormat {
    None,
    Stencil8,
}

impl StencilFormat {
    pub fn bytes_per_sample(self) -> u64 {
        match self {
            StencilFormat::None => 0,
            StencilFormat::Stencil8 => 1,
        }
    }
}

/// Why a drawable configuration is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawableError {
    /// The colour format is `None`.
    NoColorBuffer,
    /// The sample count is not 1, 2, 4 or 8. Carries the count given.
    BadSampleCount(u32),
}

/// The drawable of a view.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawable {
    pub color: ColorFormat,
    pub depth: DepthFormat,
    pub stencil: StencilFormat,
    pub samples: u32,
    pub width: u32,
    pub height: u32,
}

impl Drawable {
    /// A single-sample drawable with a colour buffer of `color`, no depth or stencil, and no size yet.
    pub fn new(color: ColorFormat) -> Self {
        Drawable {
            color,
            depth: DepthFormat::None,
            stencil: StencilFormat::None,
            samples: 1,
            width: 0,
            height: 0,
        }
    }

    /// Checks the configuration: a colour buffer must exist, and the sample count must be 1, 2, 4 or 8.
    pub fn validate(&self) -> Result<(), DrawableError> {
        if self.color == ColorFormat::None {
            return Err(DrawableError::NoColorBuffer);
        }
        match self.samples {
            1 | 2 | 4 | 8 => Ok(()),
            n => Err(DrawableError::BadSampleCount(n)),
        }
    }

    /// The bytes all buffers take at the current size, after validating. A drawable with no size takes nothing.
    pub fn memory_bytes(&self) -> Result<u64, DrawableError> {
        self.validate()?;
        let pixels = self.width as u64 * self.height as u64;
        let per_pixel = self.color.bytes_per_sample() * self.samples as u64
            + self.depth.bytes_per_sample() * self.samples as u64
            + self.stencil.bytes_per_sample() * self.samples as u64;
        Ok(pixels * per_pixel)
    }

    /// Sets the size in pixels. Width and height are taken as given.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sample_colour_only_is_four_bytes_a_pixel() {
        let mut d = Drawable::new(ColorFormat::Rgba8);
        d.resize(1920, 1080);
        assert_eq!(d.memory_bytes(), Ok(4 * 1920 * 1080));
    }

    #[test]
    fn depth_and_stencil_add_their_bytes_per_pixel() {
        // Colour 4 bytes, depth 4 bytes, stencil 1 byte: 9 bytes a pixel over 100 x 50 pixels.
        let mut d = Drawable::new(ColorFormat::Rgba8);
        d.depth = DepthFormat::Depth24;
        d.stencil = StencilFormat::Stencil8;
        d.resize(100, 50);
        assert_eq!(d.memory_bytes(), Ok(9 * 5000));
    }

    #[test]
    fn multisampling_multiplies_colour_and_depth_by_the_sample_count() {
        // A 4x drawable at 10 x 10: colour 4 * 4 bytes and depth 4 * 4 bytes a pixel, over 100 pixels.
        let mut d = Drawable::new(ColorFormat::Rgba8);
        d.depth = DepthFormat::Depth24;
        d.samples = 4;
        d.resize(10, 10);
        assert_eq!(d.memory_bytes(), Ok(32 * 100));
    }

    #[test]
    fn a_sample_count_outside_one_two_four_eight_is_refused() {
        let mut d = Drawable::new(ColorFormat::Rgb565);
        d.samples = 3;
        assert_eq!(d.validate(), Err(DrawableError::BadSampleCount(3)));
        d.samples = 8;
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn a_drawable_needs_a_colour_buffer() {
        let d = Drawable::new(ColorFormat::None);
        assert_eq!(d.validate(), Err(DrawableError::NoColorBuffer));
        assert_eq!(d.memory_bytes(), Err(DrawableError::NoColorBuffer));
    }

    #[test]
    fn a_drawable_with_no_size_takes_no_memory_and_16_bit_colour_is_two_bytes() {
        let d = Drawable::new(ColorFormat::Rgb565);
        assert_eq!(d.memory_bytes(), Ok(0));
        let mut sized = Drawable::new(ColorFormat::Rgb565);
        sized.resize(8, 4);
        assert_eq!(sized.memory_bytes(), Ok(2 * 32));
    }
}
