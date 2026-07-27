//! Checked pixel dimensions that participate in render and artifact identity.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

const MAX_VIEWPORT_EDGE: u32 = 8_192;

/// Pixel-affecting output facts owned by one render unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RenderProfile {
    width: u32,
    height: u32,
    alpha: AlphaMode,
}

impl<'de> Deserialize<'de> for RenderProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RenderProfileWire::deserialize(deserializer)?;
        Self::new(wire.width, wire.height)
            .map(|profile| profile.with_alpha(wire.alpha))
            .map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RenderProfileWire {
    width: u32,
    height: u32,
    alpha: AlphaMode,
}

/// Alpha contract carried by render units and frame-artifact identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaMode {
    /// Chromium composites authored pixels over its opaque default surface.
    Opaque,
    /// Chromium preserves authored alpha through capture and final encoding.
    Preserve,
}

impl RenderProfile {
    /// Creates checked pixel dimensions shared by every output profile.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderProfile`] when a dimension is empty, exceeds the
    /// supported viewport edge, or cannot enter every admitted pixel format.
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
        Ok(Self {
            width,
            height,
            alpha: AlphaMode::Opaque,
        })
    }

    /// Selects whether capture and encoding preserve the authored alpha plane.
    #[must_use]
    pub const fn with_alpha(mut self, alpha: AlphaMode) -> Self {
        self.alpha = alpha;
        self
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

    /// Returns the alpha contract included in portable capture identity.
    #[must_use]
    pub const fn alpha(self) -> AlphaMode {
        self.alpha
    }
}

/// Reason pixel-affecting output facts cannot enter the render profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidRenderProfile {
    /// At least one output dimension is zero.
    EmptyDimensions,
    /// At least one output dimension exceeds the supported viewport edge.
    DimensionsTooLarge,
    /// The admitted subsampled pixel formats require even dimensions.
    OddDimensions,
}

impl fmt::Display for InvalidRenderProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyDimensions => "render dimensions must be positive",
            Self::DimensionsTooLarge => "render dimensions exceed the supported viewport",
            Self::OddDimensions => "encoded video output requires even dimensions",
        })
    }
}

impl Error for InvalidRenderProfile {}

#[cfg(test)]
mod tests {
    use super::{AlphaMode, InvalidRenderProfile, MAX_VIEWPORT_EDGE, RenderProfile};

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

    #[test]
    fn makes_alpha_preservation_an_explicit_pixel_fact() {
        let profile = RenderProfile::new(320, 180)
            .expect("the output dimensions are valid")
            .with_alpha(AlphaMode::Preserve);

        assert_eq!(profile.alpha(), AlphaMode::Preserve);
        assert_eq!(
            serde_json::to_string(&profile).expect("the profile must serialize"),
            r#"{"width":320,"height":180,"alpha":"preserve"}"#,
        );
        assert_eq!(
            serde_json::from_str::<RenderProfile>(
                r#"{"width":320,"height":180,"alpha":"preserve"}"#,
            )
            .expect("the serialized profile must decode"),
            profile,
        );
    }
}
