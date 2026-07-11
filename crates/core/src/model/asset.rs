use super::Duration;

/// Normalized facts about one frozen media artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssetMetadata {
    duration: Duration,
}

impl AssetMetadata {
    /// Creates the Gate-one metadata consumed by timeline solving.
    #[must_use]
    pub const fn new(duration: Duration) -> Self {
        Self { duration }
    }

    /// Returns the exact probed artifact duration.
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.duration
    }
}
