use std::error::Error;
use std::fmt;

const MAX_VIEWPORT_EDGE: u32 = 8_192;

/// Pixel-affecting output facts owned by one render unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderProfile {
    width: u32,
    height: u32,
}

impl RenderProfile {
    /// Creates the fixed Gate-one H.264 output profile.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderProfile`] when a dimension is empty, exceeds the
    /// supported viewport edge, or cannot enter the fixed `yuv420p` encoder.
    pub const fn new(width: u32, height: u32) -> Result<Self, InvalidRenderProfile> {
        if width == 0 || height == 0 {
            return Err(InvalidRenderProfile::EmptyDimensions);
        }
        if width > MAX_VIEWPORT_EDGE || height > MAX_VIEWPORT_EDGE {
            return Err(InvalidRenderProfile::DimensionsTooLarge);
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(InvalidRenderProfile::OddDimensions);
        }
        Ok(Self { width, height })
    }

    /// Returns the viewport and encoded width in CSS pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the viewport and encoded height in CSS pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Reason pixel-affecting output facts cannot enter Gate one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidRenderProfile {
    /// At least one output dimension is zero.
    EmptyDimensions,
    /// At least one output dimension exceeds the supported viewport edge.
    DimensionsTooLarge,
    /// The fixed `yuv420p` encoder requires even dimensions.
    OddDimensions,
}

impl fmt::Display for InvalidRenderProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyDimensions => "render dimensions must be positive",
            Self::DimensionsTooLarge => "render dimensions exceed the supported viewport",
            Self::OddDimensions => "H.264 yuv420p output requires even dimensions",
        })
    }
}

impl Error for InvalidRenderProfile {}

#[cfg(test)]
mod tests {
    use super::{InvalidRenderProfile, MAX_VIEWPORT_EDGE, RenderProfile};

    #[test]
    fn owns_valid_output_dimensions() {
        let profile = RenderProfile::new(1_920, 1_080).expect("the output dimensions are valid");

        assert_eq!(profile.width(), 1_920);
        assert_eq!(profile.height(), 1_080);
        assert_eq!(
            RenderProfile::new(0, 180),
            Err(InvalidRenderProfile::EmptyDimensions),
        );
        assert_eq!(
            RenderProfile::new(MAX_VIEWPORT_EDGE + 1, 180),
            Err(InvalidRenderProfile::DimensionsTooLarge),
        );
        assert_eq!(
            RenderProfile::new(321, 180),
            Err(InvalidRenderProfile::OddDimensions),
        );
    }
}
