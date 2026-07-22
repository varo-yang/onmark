//! Closed presentation-visual capabilities used by render execution.

use std::fmt;
use std::str::FromStr;

/// Proven relationship between browser presentation pixels and primary media.
///
/// Unknown presentation code is [`BrowserComposite`](Self::BrowserComposite).
/// A separable overlay is an explicit claim that the browser output is a
/// transparent foreground independent of the primary video beneath it.
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

#[cfg(test)]
mod tests {
    use super::PresentationVisualCapability;

    #[test]
    fn canonical_spellings_round_trip() {
        for capability in [
            PresentationVisualCapability::BrowserComposite,
            PresentationVisualCapability::SeparableOverlay,
        ] {
            assert_eq!(capability.as_str().parse(), Ok(capability));
        }
    }
}
