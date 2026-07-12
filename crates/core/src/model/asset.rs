use std::fmt;

use super::Duration;

/// Byte width of the Gate-one SHA-256 asset digest.
const SHA256_BYTES: usize = 32;

/// Immutable identity of the exact asset bytes consumed by compilation.
///
/// Paths and authored references may change between machines. This identity
/// crosses into Timeline IR so later materialization can prove it supplied the
/// bytes whose metadata the compiler used.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrozenAssetId([u8; SHA256_BYTES]);

impl FrozenAssetId {
    /// Creates an asset identity from a SHA-256 digest computed while freezing
    /// the input bytes.
    #[must_use]
    pub const fn from_sha256(digest: [u8; SHA256_BYTES]) -> Self {
        Self(digest)
    }

    /// Returns the SHA-256 digest bytes.
    #[must_use]
    pub const fn as_sha256(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }
}

impl fmt::Display for FrozenAssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.as_sha256() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Normalized facts probed from one media artifact.
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

/// One frozen artifact and the normalized facts probed from those same bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenAsset {
    id: FrozenAssetId,
    metadata: AssetMetadata,
}

impl FrozenAsset {
    /// Joins immutable byte identity with metadata derived from those bytes.
    ///
    /// The IO boundary constructing this value must ensure that `metadata` was
    /// probed from the bytes identified by `id`; pure core cannot inspect that
    /// external fact.
    #[must_use]
    pub const fn new(id: FrozenAssetId, metadata: AssetMetadata) -> Self {
        Self { id, metadata }
    }

    /// Returns the immutable artifact identity.
    #[must_use]
    pub const fn id(self) -> FrozenAssetId {
        self.id
    }

    /// Returns normalized probe facts for the immutable artifact.
    #[must_use]
    pub const fn metadata(self) -> AssetMetadata {
        self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::FrozenAssetId;

    #[test]
    fn frozen_identity_has_an_algorithm_named_canonical_spelling() {
        let id = FrozenAssetId::from_sha256([0xab; 32]);

        assert_eq!(
            id.to_string(),
            "sha256:abababababababababababababababababababababababababababababababab",
        );
        assert_eq!(id.as_sha256(), &[0xab; 32]);
    }
}
