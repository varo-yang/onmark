//! Opaque identity for the host facts capable of changing captured pixels.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const SHA256_BYTES: usize = 32;

/// Immutable identity of one fully locked browser capture environment.
///
/// This digest is supplied by the deployment that owns Chromium, fonts, launch
/// configuration, and other pixel-affecting host facts. The renderer does not
/// guess a partial identity from an executable path or browser version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CaptureEnvironmentId([u8; SHA256_BYTES]);

impl CaptureEnvironmentId {
    /// Number of SHA-256 bytes in one capture-environment identity.
    pub const BYTE_LENGTH: usize = SHA256_BYTES;

    /// Creates an identity from a deployment-computed SHA-256 digest.
    #[must_use]
    pub const fn from_sha256(digest: [u8; SHA256_BYTES]) -> Self {
        Self(digest)
    }

    /// Parses the canonical `sha256:<lowercase-hex>` spelling.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCaptureEnvironmentId`] when the prefix, digest length,
    /// or hexadecimal spelling is not canonical.
    pub fn parse(value: &str) -> Result<Self, InvalidCaptureEnvironmentId> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(InvalidCaptureEnvironmentId::MissingPrefix);
        };
        if hex.len() != SHA256_BYTES * 2 {
            return Err(InvalidCaptureEnvironmentId::InvalidLength);
        }

        let mut digest = [0; SHA256_BYTES];
        for (index, byte) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            let high = hex_value(hex.as_bytes()[offset])?;
            let low = hex_value(hex.as_bytes()[offset + 1])?;
            *byte = high << 4 | low;
        }
        Ok(Self::from_sha256(digest))
    }

    /// Returns the deployment-computed SHA-256 digest bytes.
    #[must_use]
    pub const fn as_sha256(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CaptureEnvironmentId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Box::<str>::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

impl Serialize for CaptureEnvironmentId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Reason a capture environment cannot name one immutable deployment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCaptureEnvironmentId {
    /// The required `sha256:` prefix is absent.
    MissingPrefix,
    /// The digest does not have exactly 64 hexadecimal characters.
    InvalidLength,
    /// The digest contains a noncanonical hexadecimal byte.
    InvalidHex,
}

impl fmt::Display for InvalidCaptureEnvironmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPrefix => "capture environment identity must start with sha256:",
            Self::InvalidLength => {
                "capture environment identity must contain 64 hexadecimal characters"
            }
            Self::InvalidHex => {
                "capture environment identity must use lowercase hexadecimal characters"
            }
        })
    }
}

impl Error for InvalidCaptureEnvironmentId {}

impl fmt::Display for CaptureEnvironmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.as_sha256() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn hex_value(byte: u8) -> Result<u8, InvalidCaptureEnvironmentId> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(InvalidCaptureEnvironmentId::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureEnvironmentId, InvalidCaptureEnvironmentId};

    #[test]
    fn parses_only_canonical_capture_environment_identities() {
        let identity = CaptureEnvironmentId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("the canonical environment identity parses");

        assert_eq!(
            identity.to_string(),
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );
        assert_eq!(
            CaptureEnvironmentId::parse(
                "SHA256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            ),
            Err(InvalidCaptureEnvironmentId::MissingPrefix),
        );
        assert_eq!(
            CaptureEnvironmentId::parse(
                "sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
            ),
            Err(InvalidCaptureEnvironmentId::InvalidHex),
        );
    }

    #[test]
    fn serializes_as_its_canonical_identity() {
        let identity = CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH]);
        let encoded =
            serde_json::to_string(&identity).expect("the canonical capture environment serializes");
        let decoded: CaptureEnvironmentId =
            serde_json::from_str(&encoded).expect("the canonical capture environment parses");

        assert_eq!(
            encoded,
            "\"sha256:0707070707070707070707070707070707070707070707070707070707070707\""
        );
        assert_eq!(decoded, identity);
    }
}
