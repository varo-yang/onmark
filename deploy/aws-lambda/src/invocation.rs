//! Versioned AWS invocation and result contracts over portable worker values.
//!
//! S3 locations are deployment facts; render-engine and AWS SDK types never
//! enter each other's domains.

use std::error::Error;
use std::fmt;

use onmark_render::FrameArtifactId;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

#[cfg(any(feature = "runtime", test))]
const ARTIFACT_DIRECTORY: &str = "frame-artifacts";

/// Version of the Lambda invocation contract.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CaptureInvocationVersion(u16);

impl CaptureInvocationVersion {
    /// First version of the immutable frame-capture invocation.
    pub const V1: Self = Self(1);

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for CaptureInvocationVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == Self::V1.get() {
            return Ok(Self::V1);
        }
        Err(D::Error::custom("unsupported AWS Lambda capture version"))
    }
}

/// One S3 bucket and canonical object-key prefix.
///
/// Prefixes name remote namespaces only. They never become local paths, so
/// request-controlled spelling cannot escape the handler's private `/tmp`
/// workspace.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ObjectPrefix {
    #[cfg_attr(feature = "schema", schemars(length(min = 1), regex(pattern = r"\S")))]
    bucket: Box<str>,
    #[cfg_attr(
        feature = "schema",
        schemars(regex(
            pattern = r"^(?:$|(?!/)(?!(?:.*/)?(?:\.|\.\.)(?:/|$))(?!.*//)(?!.*\/$).+)$"
        ))
    )]
    prefix: Box<str>,
}

impl ObjectPrefix {
    /// Creates one canonical object-store namespace.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidObjectPrefix`] when the bucket is blank or the prefix
    /// has ambiguous separators or traversal segments.
    pub fn new(
        bucket: impl Into<Box<str>>,
        prefix: impl Into<Box<str>>,
    ) -> Result<Self, InvalidObjectPrefix> {
        let bucket = bucket.into();
        if bucket.trim().is_empty() {
            return Err(InvalidObjectPrefix::BlankBucket);
        }
        let prefix = prefix.into();
        if !is_canonical_prefix(&prefix) {
            return Err(InvalidObjectPrefix::InvalidPrefix);
        }

        Ok(Self { bucket, prefix })
    }

    /// Returns the S3 bucket name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Returns the canonical object-key prefix, possibly empty.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Returns one object key beneath this namespace.
    #[must_use]
    #[cfg(any(feature = "runtime", test))]
    pub(crate) fn key(&self, suffix: &str) -> String {
        if self.prefix.is_empty() {
            return suffix.to_owned();
        }
        format!("{}/{suffix}", self.prefix)
    }
}

impl<'de> Deserialize<'de> for ObjectPrefix {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ObjectPrefixWire::deserialize(deserializer)?;
        Self::new(wire.bucket, wire.prefix).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ObjectPrefixWire {
    bucket: Box<str>,
    prefix: Box<str>,
}

/// One immutable worker input published beneath an S3 prefix.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureInvocation {
    version: CaptureInvocationVersion,
    input: ObjectPrefix,
}

impl CaptureInvocation {
    /// Creates one request for the canonical worker-input layout.
    #[must_use]
    pub fn new(input: ObjectPrefix) -> Self {
        Self {
            version: CaptureInvocationVersion::V1,
            input,
        }
    }

    /// Returns the invocation contract version.
    #[must_use]
    pub const fn version(&self) -> CaptureInvocationVersion {
        self.version
    }

    /// Returns the object namespace containing request and frozen payloads.
    #[must_use]
    pub const fn input(&self) -> &ObjectPrefix {
        &self.input
    }
}

impl<'de> Deserialize<'de> for CaptureInvocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CaptureInvocationWire::deserialize(deserializer)?;
        Ok(Self::new(wire.input))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaptureInvocationWire {
    #[serde(rename = "version")]
    _version: CaptureInvocationVersion,
    input: ObjectPrefix,
}

/// One immutable frame artifact reachable through S3.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactLocation {
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    bucket: Box<str>,
    #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
    key: Box<str>,
    artifact_id: FrameArtifactId,
    #[cfg_attr(feature = "schema", schemars(range(min = 1)))]
    frames: u64,
}

impl ArtifactLocation {
    #[cfg(feature = "runtime")]
    pub(crate) fn new(
        bucket: impl Into<Box<str>>,
        key: impl Into<Box<str>>,
        artifact_id: FrameArtifactId,
        frames: u64,
    ) -> Self {
        Self {
            bucket: bucket.into(),
            key: key.into(),
            artifact_id,
            frames,
        }
    }

    /// Returns the artifact bucket.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Returns the immutable artifact object key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the deterministic capture-contract identity from the request.
    #[must_use]
    pub const fn artifact_id(&self) -> FrameArtifactId {
        self.artifact_id
    }

    /// Returns the ordered output-frame count verified by the artifact.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }
}

/// Whether this invocation committed a new object or reused a verified one.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Publication {
    /// This invocation conditionally committed a new immutable artifact.
    Published,
    /// A concurrent or prior invocation had already committed the same artifact.
    Reused,
}

/// Structured result of one frame-capture invocation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureResult {
    version: CaptureInvocationVersion,
    artifact: ArtifactLocation,
    publication: Publication,
}

impl CaptureResult {
    #[cfg(feature = "runtime")]
    pub(crate) fn new(artifact: ArtifactLocation, publication: Publication) -> Self {
        Self {
            version: CaptureInvocationVersion::V1,
            artifact,
            publication,
        }
    }

    /// Returns the result contract version.
    #[must_use]
    pub const fn version(&self) -> CaptureInvocationVersion {
        self.version
    }

    /// Returns the immutable artifact location.
    #[must_use]
    pub const fn artifact(&self) -> &ArtifactLocation {
        &self.artifact
    }

    /// Returns whether this invocation published or reused the object.
    #[must_use]
    pub const fn publication(&self) -> Publication {
        self.publication
    }
}

/// Reason an S3 namespace cannot name the worker-input layout unambiguously.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidObjectPrefix {
    /// The S3 bucket name has no non-whitespace characters.
    BlankBucket,
    /// The prefix uses leading, trailing, repeated, or traversal separators.
    InvalidPrefix,
}

impl fmt::Display for InvalidObjectPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BlankBucket => "S3 bucket must not be blank",
            Self::InvalidPrefix => "S3 prefix must use canonical non-traversing segments",
        })
    }
}

impl Error for InvalidObjectPrefix {}

#[cfg(any(feature = "runtime", test))]
pub(crate) fn artifact_key(destination: &ObjectPrefix, id: FrameArtifactId) -> String {
    destination.key(&format!("{ARTIFACT_DIRECTORY}/{id}.onmark-frames"))
}

fn is_canonical_prefix(prefix: &str) -> bool {
    prefix.is_empty()
        || (!prefix.starts_with('/')
            && !prefix.ends_with('/')
            && prefix
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != ".."))
}

#[cfg(test)]
mod tests {
    use onmark_render::FrameArtifactId;

    use super::{CaptureInvocation, CaptureInvocationVersion, ObjectPrefix, artifact_key};

    #[test]
    fn keeps_input_and_artifact_object_namespaces_disjoint() {
        let input = ObjectPrefix::new("onmark-inputs", "captures/film-a")
            .expect("the fixture prefix is canonical");
        let artifacts = ObjectPrefix::new("onmark-artifacts", "production")
            .expect("the fixture prefix is canonical");
        let id = FrameArtifactId::parse(
            "sha256:abababababababababababababababababababababababababababababababab",
        )
        .expect("the fixture artifact id is canonical");

        assert_eq!(input.key("request.json"), "captures/film-a/request.json");
        assert_eq!(
            artifact_key(&artifacts, id),
            "production/frame-artifacts/sha256:abababababababababababababababababababababababababababababababab.onmark-frames"
        );
    }

    #[test]
    fn serializes_one_versioned_capture_invocation() {
        let input = ObjectPrefix::new("onmark-inputs", "captures/film-a")
            .expect("the fixture prefix is canonical");
        let invocation = CaptureInvocation::new(input);
        let encoded = serde_json::to_string(&invocation).expect("the invocation serializes");
        let decoded: CaptureInvocation =
            serde_json::from_str(&encoded).expect("the invocation parses once");

        assert_eq!(decoded, invocation);
        assert_eq!(decoded.version(), CaptureInvocationVersion::V1);
        assert_eq!(
            encoded,
            r#"{"version":1,"input":{"bucket":"onmark-inputs","prefix":"captures/film-a"}}"#
        );
    }

    #[test]
    fn rejects_ambiguous_object_prefixes() {
        assert!(ObjectPrefix::new("", "captures").is_err());
        assert!(ObjectPrefix::new("bucket", "/captures").is_err());
        assert!(ObjectPrefix::new("bucket", "captures/").is_err());
        assert!(ObjectPrefix::new("bucket", "captures//film").is_err());
        assert!(ObjectPrefix::new("bucket", "captures/../film").is_err());
    }
}
