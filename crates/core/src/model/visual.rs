//! Closed presentation-artifact facts used by render execution.

use std::fmt;
use std::str::FromStr;

/// Semantic DOM extent present in one immutable browser artifact.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationDocumentScope {
    /// The browser document contains the complete authored film.
    #[default]
    WholeFilm,
    /// The browser document contains exactly one independently planned region.
    RenderRegion,
}

impl PresentationDocumentScope {
    /// Returns the canonical wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WholeFilm => "wholeFilm",
            Self::RenderRegion => "renderRegion",
        }
    }
}

impl fmt::Display for PresentationDocumentScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PresentationDocumentScope {
    type Err = InvalidPresentationDocumentScope;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "wholeFilm" => Ok(Self::WholeFilm),
            "renderRegion" => Ok(Self::RenderRegion),
            _ => Err(InvalidPresentationDocumentScope),
        }
    }
}

/// Reason a presentation document-scope spelling was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPresentationDocumentScope;

impl fmt::Display for InvalidPresentationDocumentScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected wholeFilm or renderRegion")
    }
}

impl std::error::Error for InvalidPresentationDocumentScope {}

/// Proven relationship between browser presentation pixels and primary media.
///
/// Unknown presentation code requires `BrowserComposite`. A separable layer
/// explicitly limits browser ownership to one side of native media.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationVisualCapability {
    /// Chromium owns the complete frame, including primary media.
    #[default]
    BrowserComposite,
    /// Chromium owns only the browser backdrop beneath native media.
    SeparableBackdrop,
    /// Chromium owns only a transparent foreground over native primary media.
    SeparableOverlay,
}

impl PresentationVisualCapability {
    /// Returns the canonical wire and command-line spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserComposite => "browserComposite",
            Self::SeparableBackdrop => "separableBackdrop",
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
            "separableBackdrop" => Ok(Self::SeparableBackdrop),
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
        formatter.write_str("expected browserComposite, separableBackdrop, or separableOverlay")
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
    use super::{
        PresentationDocumentScope, PresentationFrameBehavior, PresentationVisualCapability,
    };

    #[test]
    fn document_scope_spellings_round_trip() {
        for scope in [
            PresentationDocumentScope::WholeFilm,
            PresentationDocumentScope::RenderRegion,
        ] {
            assert_eq!(scope.as_str().parse(), Ok(scope));
        }
    }

    #[test]
    fn canonical_spellings_round_trip() {
        for capability in [
            PresentationVisualCapability::BrowserComposite,
            PresentationVisualCapability::SeparableBackdrop,
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
