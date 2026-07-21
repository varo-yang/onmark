//! Closed presentation-time capabilities used by deterministic planning.

use std::fmt;
use std::str::FromStr;

/// Proven relationship between a presentation's output and requested frames.
///
/// Unknown presentation code is [`Sequential`](Self::Sequential). Random
/// access is an explicit claim that each requested frame depends only on
/// immutable inputs and that exact frame.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PresentationTemporalCapability {
    /// Frames must be evaluated as one screenplay-ordered sequence.
    #[default]
    Sequential,
    /// Any requested frame can be evaluated independently.
    RandomAccess,
}

impl PresentationTemporalCapability {
    /// Returns the canonical wire and command-line spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::RandomAccess => "randomAccess",
        }
    }
}

impl fmt::Display for PresentationTemporalCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PresentationTemporalCapability {
    type Err = InvalidPresentationTemporalCapability;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sequential" => Ok(Self::Sequential),
            "randomAccess" => Ok(Self::RandomAccess),
            _ => Err(InvalidPresentationTemporalCapability),
        }
    }
}

/// Reason a presentation temporal capability spelling was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPresentationTemporalCapability;

impl fmt::Display for InvalidPresentationTemporalCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected sequential or randomAccess")
    }
}

impl std::error::Error for InvalidPresentationTemporalCapability {}

#[cfg(test)]
mod tests {
    use super::PresentationTemporalCapability;

    #[test]
    fn canonical_spellings_round_trip() {
        for capability in [
            PresentationTemporalCapability::Sequential,
            PresentationTemporalCapability::RandomAccess,
        ] {
            assert_eq!(capability.as_str().parse(), Ok(capability));
        }
    }
}
