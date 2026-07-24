//! Closed presentation-visual capabilities used by render execution.

use std::fmt;
use std::str::FromStr;

/// Proven relationship between browser presentation pixels and primary media.
///
/// Unknown presentation code requires `BrowserComposite`. A separable overlay
/// is an explicit claim that the browser output is a transparent foreground
/// independent of the primary video beneath it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationVisualCapability {
    /// Chromium owns the complete frame, including primary media.
    #[default]
    BrowserComposite,
    /// Chromium owns only a transparent foreground over native primary media.
    SeparableOverlay,
}

impl PresentationVisualCapability {
    /// Returns the canonical wire and command-line spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserComposite => "browserComposite",
            Self::SeparableOverlay => "separableOverlay",
        }
    }
}

impl fmt::Display for PresentationVisualCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PresentationVisualCapability {
    type Err = InvalidPresentationVisualCapability;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "browserComposite" => Ok(Self::BrowserComposite),
            "separableOverlay" => Ok(Self::SeparableOverlay),
            _ => Err(InvalidPresentationVisualCapability),
        }
    }
}

/// Reason a presentation visual-capability spelling was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPresentationVisualCapability;

impl fmt::Display for InvalidPresentationVisualCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected browserComposite or separableOverlay")
    }
}

impl std::error::Error for InvalidPresentationVisualCapability {}

/// Proven cadence at which browser-owned pixels may change.
///
/// Unknown presentation code requires `PerFrame`. Placement-bounded pixels may
/// change only when the browser plan changes its active structural placements,
/// so execution may reuse one exact capture between those boundaries.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationFrameBehavior {
    /// Browser-owned pixels may differ at every authored frame.
    #[default]
    PerFrame,
    /// Browser-owned pixels are constant between placement boundaries.
    PlacementBounded,
}

impl PresentationFrameBehavior {
    /// Returns the canonical wire and command-line spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PerFrame => "perFrame",
            Self::PlacementBounded => "placementBounded",
        }
    }
}

impl fmt::Display for PresentationFrameBehavior {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PresentationFrameBehavior {
    type Err = InvalidPresentationFrameBehavior;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "perFrame" => Ok(Self::PerFrame),
            "placementBounded" => Ok(Self::PlacementBounded),
            _ => Err(InvalidPresentationFrameBehavior),
        }
    }
}

/// Reason a presentation frame-behavior spelling was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPresentationFrameBehavior;

impl fmt::Display for InvalidPresentationFrameBehavior {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected perFrame or placementBounded")
    }
}

impl std::error::Error for InvalidPresentationFrameBehavior {}

#[cfg(test)]
mod tests {
    use super::{PresentationFrameBehavior, PresentationVisualCapability};

    #[test]
    fn canonical_spellings_round_trip() {
        for capability in [
            PresentationVisualCapability::BrowserComposite,
            PresentationVisualCapability::SeparableOverlay,
        ] {
            assert_eq!(capability.as_str().parse(), Ok(capability));
        }
    }

    #[test]
    fn frame_behavior_spellings_round_trip() {
        for behavior in [
            PresentationFrameBehavior::PerFrame,
            PresentationFrameBehavior::PlacementBounded,
        ] {
            assert_eq!(behavior.as_str().parse(), Ok(behavior));
        }
    }
}
