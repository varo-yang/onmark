//! Versioned wire contract for one immutable browser presentation bundle.
//!
//! Construction and deserialization share canonical path, size, ordering, and
//! content-identity validation.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{FrozenAssetId, PresentationTemporalCapability, PresentationVisualCapability};

const ENTRY_DOCUMENT: &str = "index.html";
const MANIFEST_FILE: &str = "manifest.json";
const ASSET_ROOT: &str = "assets";
const ASSET_SHA256_DIRECTORY: &str = "assets/sha256";
const MAX_BUNDLE_FILES: usize = 99_999;
const MAX_PATH_BYTES: usize = 1_024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const SHA256_PREFIX: &str = "sha256:";
const SHA256_HEX_BYTES: usize = 64;

/// Version of the immutable presentation-bundle contract.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BundleVersion(u16);

impl BundleVersion {
    /// Only bundle-manifest version accepted by this build.
    pub const CURRENT: Self = Self(1);

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for BundleVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == Self::CURRENT.get() {
            return Ok(Self::CURRENT);
        }
        Err(D::Error::custom("unsupported bundle manifest version"))
    }
}

/// Stable description of one immutable presentation artifact.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        rename_all = "camelCase",
        deny_unknown_fields,
        extend("x-onmark-manifest-file" = MANIFEST_FILE),
        extend("x-onmark-asset-directory" = ASSET_SHA256_DIRECTORY)
    )
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleManifest {
    version: BundleVersion,
    #[cfg_attr(
        feature = "schema",
        schemars(regex(pattern = r"^sha256:[0-9a-f]{64}$"))
    )]
    bundle_id: Box<str>,
    #[cfg_attr(feature = "schema", schemars(extend("const" = ENTRY_DOCUMENT)))]
    entry_point: Box<str>,
    temporal_capability: PresentationTemporalCapability,
    visual_capability: PresentationVisualCapability,
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = MAX_BUNDLE_FILES))
    )]
    files: Vec<BundleFile>,
}

impl BundleManifest {
    /// Fixed browser entry beneath every presentation bundle.
    pub const ENTRY_POINT: &'static str = ENTRY_DOCUMENT;
    /// Reserved manifest filename beneath every unit root.
    pub const FILE_NAME: &'static str = MANIFEST_FILE;
    /// Deterministic directory containing frozen SHA-256 assets.
    pub const ASSET_DIRECTORY: &'static str = ASSET_SHA256_DIRECTORY;
    /// Maximum payload files representable by the current wire contract.
    pub const MAX_FILES: usize = MAX_BUNDLE_FILES;

    /// Returns the deterministic path of one frozen asset beneath a unit root.
    #[must_use]
    pub fn asset_path(id: FrozenAssetId) -> String {
        let mut path = String::from(Self::ASSET_DIRECTORY);
        path.push('/');
        for byte in id.as_sha256() {
            write!(path, "{byte:02x}").expect("writing into a String cannot fail");
        }
        path
    }

    /// Creates one canonical current-version bundle manifest.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBundleManifest`] when the identity, file count,
    /// canonical path tree, or fixed entry document violates the contract.
    pub fn new(
        temporal_capability: PresentationTemporalCapability,
        visual_capability: PresentationVisualCapability,
        bundle_id: impl Into<Box<str>>,
        files: Vec<BundleFile>,
    ) -> Result<Self, InvalidBundleManifest> {
        Self::from_parts(
            temporal_capability,
            visual_capability,
            bundle_id.into(),
            files,
        )
    }

    fn from_parts(
        temporal_capability: PresentationTemporalCapability,
        visual_capability: PresentationVisualCapability,
        bundle_id: Box<str>,
        files: Vec<BundleFile>,
    ) -> Result<Self, InvalidBundleManifest> {
        if !is_sha256(&bundle_id) {
            return Err(InvalidBundleManifest::InvalidBundleId);
        }
        if files.is_empty() {
            return Err(InvalidBundleManifest::EmptyFiles);
        }
        if files.len() > MAX_BUNDLE_FILES {
            return Err(InvalidBundleManifest::TooManyFiles);
        }
        validate_file_order(&files)?;
        validate_path_tree(&files)?;
        if !files.iter().any(|file| file.path() == ENTRY_DOCUMENT) {
            return Err(InvalidBundleManifest::MissingEntryPoint);
        }

        Ok(Self {
            version: BundleVersion::CURRENT,
            bundle_id,
            entry_point: ENTRY_DOCUMENT.into(),
            temporal_capability,
            visual_capability,
            files,
        })
    }

    /// Returns the bundle manifest version.
    #[must_use]
    pub const fn version(&self) -> BundleVersion {
        self.version
    }

    /// Returns the content identity over the canonical payload description.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// Returns the fixed browser entry document.
    #[must_use]
    pub fn entry_point(&self) -> &str {
        &self.entry_point
    }

    /// Returns the proven temporal behavior used by render planning.
    #[must_use]
    pub const fn temporal_capability(&self) -> PresentationTemporalCapability {
        self.temporal_capability
    }

    /// Returns the proven visual relationship used by render execution.
    #[must_use]
    pub const fn visual_capability(&self) -> PresentationVisualCapability {
        self.visual_capability
    }

    /// Returns the canonical JSON identity payload whose SHA-256 names the bundle.
    #[must_use]
    pub const fn identity(&self) -> BundleIdentity<'_> {
        BundleIdentity { manifest: self }
    }

    /// Returns payload files in canonical path order.
    #[must_use]
    pub fn files(&self) -> &[BundleFile] {
        &self.files
    }
}

/// Borrowed canonical bundle-identity projection.
///
/// Its serialized field order is part of each manifest version's wire contract.
#[derive(Clone, Copy, Debug)]
pub struct BundleIdentity<'a> {
    manifest: &'a BundleManifest,
}

impl Serialize for BundleIdentity<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let manifest = self.manifest;
        let mut identity = serializer.serialize_struct("BundleIdentity", 5)?;
        identity.serialize_field("version", &manifest.version)?;
        identity.serialize_field("entryPoint", &manifest.entry_point)?;
        identity.serialize_field("temporalCapability", manifest.temporal_capability.as_str())?;
        identity.serialize_field("visualCapability", manifest.visual_capability.as_str())?;
        identity.serialize_field("files", &manifest.files)?;
        identity.end()
    }
}

impl Serialize for BundleManifest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut manifest = serializer.serialize_struct("BundleManifest", 6)?;
        manifest.serialize_field("version", &self.version)?;
        manifest.serialize_field("bundleId", &self.bundle_id)?;
        manifest.serialize_field("entryPoint", &self.entry_point)?;
        manifest.serialize_field("temporalCapability", self.temporal_capability.as_str())?;
        manifest.serialize_field("visualCapability", self.visual_capability.as_str())?;
        manifest.serialize_field("files", &self.files)?;
        manifest.end()
    }
}

impl<'de> Deserialize<'de> for BundleManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let BundleManifestWire {
            version: _version,
            bundle_id,
            entry_point,
            temporal_capability,
            visual_capability,
            files,
        } = BundleManifestWire::deserialize(deserializer)?;
        if entry_point.as_ref() != ENTRY_DOCUMENT {
            return Err(D::Error::custom("unsupported bundle entry point"));
        }
        let temporal_capability = temporal_capability
            .parse()
            .map_err(|_| D::Error::custom("invalid bundle temporal capability"))?;
        let visual_capability = visual_capability
            .parse()
            .map_err(|_| D::Error::custom("invalid bundle visual capability"))?;
        Self::from_parts(temporal_capability, visual_capability, bundle_id, files)
            .map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BundleManifestWire {
    version: BundleVersion,
    bundle_id: Box<str>,
    entry_point: Box<str>,
    temporal_capability: Box<str>,
    visual_capability: Box<str>,
    files: Vec<BundleFile>,
}

/// One content-addressed presentation payload file.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BundleFile {
    // Declaration order is part of the canonical bundle-identity encoding.
    #[cfg_attr(feature = "schema", schemars(range(max = MAX_SAFE_INTEGER)))]
    bytes: u64,
    #[cfg_attr(
        feature = "schema",
        schemars(
            length(max = MAX_PATH_BYTES),
            regex(pattern = r"^[a-z0-9._-]+(?:/[a-z0-9._-]+)*$")
        )
    )]
    path: Box<str>,
    #[cfg_attr(
        feature = "schema",
        schemars(regex(pattern = r"^sha256:[0-9a-f]{64}$"))
    )]
    sha256: Box<str>,
}

impl BundleFile {
    /// Creates one validated payload entry.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBundleFile`] when the path could escape or collide
    /// with unit-owned files, the size is not browser-safe, or the digest is
    /// not canonical SHA-256.
    pub fn new(
        path: impl Into<Box<str>>,
        bytes: u64,
        sha256: impl Into<Box<str>>,
    ) -> Result<Self, InvalidBundleFile> {
        let path = path.into();
        validate_path(&path)?;
        if bytes > MAX_SAFE_INTEGER {
            return Err(InvalidBundleFile::UnsafeByteCount);
        }
        let sha256 = sha256.into();
        if !is_sha256(&sha256) {
            return Err(InvalidBundleFile::InvalidDigest);
        }

        Ok(Self {
            bytes,
            path,
            sha256,
        })
    }

    /// Returns the portable path relative to the bundle root.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the exact retained byte count.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Returns the canonical SHA-256 identity.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

impl<'de> Deserialize<'de> for BundleFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BundleFileWire::deserialize(deserializer)?;
        Self::new(wire.path, wire.bytes, wire.sha256).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BundleFileWire {
    path: Box<str>,
    bytes: u64,
    sha256: Box<str>,
}

/// Reason a bundle manifest cannot become trusted execution input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBundleManifest {
    /// The bundle identity is not canonical SHA-256.
    InvalidBundleId,
    /// No presentation payload files were supplied.
    EmptyFiles,
    /// The payload file count exceeds the manifest safety ceiling.
    TooManyFiles,
    /// Two files claim the same portable path.
    DuplicateFilePath,
    /// Files are not in canonical path order.
    UnorderedFiles,
    /// One file path is an ancestor of another file path.
    FilePathConflict,
    /// The fixed browser entry document is absent.
    MissingEntryPoint,
}

impl fmt::Display for InvalidBundleManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBundleId => "bundle ID is not canonical SHA-256",
            Self::EmptyFiles => "bundle manifest must contain payload files",
            Self::TooManyFiles => "bundle manifest exceeds the file-count ceiling",
            Self::DuplicateFilePath => "bundle manifest contains a duplicate file path",
            Self::UnorderedFiles => "bundle manifest files are not in canonical path order",
            Self::FilePathConflict => "bundle manifest contains a file-directory path collision",
            Self::MissingEntryPoint => "bundle manifest does not contain index.html",
        })
    }
}

impl Error for InvalidBundleManifest {}

/// Reason one bundle payload entry is unsafe or non-canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBundleFile {
    /// The path is not a safe portable relative path.
    InvalidPath,
    /// The portable path exceeds the manifest byte ceiling.
    PathTooLong,
    /// The path collides with unit-root owned content.
    ReservedPath,
    /// The byte count cannot cross the JavaScript boundary exactly.
    UnsafeByteCount,
    /// The content identity is not canonical SHA-256.
    InvalidDigest,
}

impl fmt::Display for InvalidBundleFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPath => "bundle file path is not a safe portable relative path",
            Self::PathTooLong => "bundle file path exceeds the byte ceiling",
            Self::ReservedPath => "bundle file path is reserved by the unit root",
            Self::UnsafeByteCount => "bundle file byte count exceeds the safe integer range",
            Self::InvalidDigest => "bundle file digest is not canonical SHA-256",
        })
    }
}

impl Error for InvalidBundleFile {}

fn validate_path(path: &str) -> Result<(), InvalidBundleFile> {
    if path.len() > MAX_PATH_BYTES {
        return Err(InvalidBundleFile::PathTooLong);
    }
    let mut segments = path.split('/');
    let Some(first) = segments.next().filter(|segment| valid_segment(segment)) else {
        return Err(InvalidBundleFile::InvalidPath);
    };
    if segments.any(|segment| !valid_segment(segment)) {
        return Err(InvalidBundleFile::InvalidPath);
    }
    if first == MANIFEST_FILE || first == ASSET_ROOT {
        return Err(InvalidBundleFile::ReservedPath);
    }
    Ok(())
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.ends_with('.')
        && !is_windows_device_name(segment)
        && segment.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn is_windows_device_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|number| matches!(number.as_bytes(), [b'1'..=b'9']))
}

fn validate_file_order(files: &[BundleFile]) -> Result<(), InvalidBundleManifest> {
    for pair in files.windows(2) {
        if pair[0].path() == pair[1].path() {
            return Err(InvalidBundleManifest::DuplicateFilePath);
        }
        if pair[0].path() > pair[1].path() {
            return Err(InvalidBundleManifest::UnorderedFiles);
        }
    }
    Ok(())
}

fn validate_path_tree(files: &[BundleFile]) -> Result<(), InvalidBundleManifest> {
    let mut paths = BTreeSet::new();
    for file in files {
        // Canonical ordering guarantees that every possible file ancestor has
        // already appeared, even when unrelated siblings separate the pair.
        for (separator, _) in file.path().match_indices('/') {
            if paths.contains(&file.path()[..separator]) {
                return Err(InvalidBundleManifest::FilePathConflict);
            }
        }
        paths.insert(file.path());
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    let Some(digest) = value.strip_prefix(SHA256_PREFIX) else {
        return false;
    };
    digest.len() == SHA256_HEX_BYTES
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use crate::model::{PresentationTemporalCapability, PresentationVisualCapability};

    use super::{BundleFile, BundleManifest, InvalidBundleFile, InvalidBundleManifest};

    const DIGEST: &str = "sha256:0101010101010101010101010101010101010101010101010101010101010101";

    #[test]
    fn accepts_one_canonical_entry_document() {
        let file = BundleFile::new("index.html", 12, DIGEST).expect("the fixture file is valid");
        let manifest = BundleManifest::new(
            PresentationTemporalCapability::Sequential,
            PresentationVisualCapability::BrowserComposite,
            DIGEST,
            vec![file],
        )
        .expect("the fixture manifest is canonical");

        assert_eq!(manifest.entry_point(), "index.html");
        assert_eq!(manifest.files().len(), 1);
    }

    #[test]
    fn rejects_unsafe_or_reserved_paths() {
        for path in [
            "../index.html",
            "/index.html",
            "a//b",
            "a\\b",
            "space name",
            "Upper.js",
            "trailing.",
            "nul.txt",
        ] {
            assert_eq!(
                BundleFile::new(path, 1, DIGEST),
                Err(InvalidBundleFile::InvalidPath),
            );
        }
        for path in ["manifest.json", "assets/video.mp4"] {
            assert_eq!(
                BundleFile::new(path, 1, DIGEST),
                Err(InvalidBundleFile::ReservedPath),
            );
        }

        assert_eq!(
            BundleFile::new("manifest.json/child", 1, DIGEST),
            Err(InvalidBundleFile::ReservedPath),
        );
        assert_eq!(
            BundleFile::new("a".repeat(1_025), 1, DIGEST),
            Err(InvalidBundleFile::PathTooLong),
        );
    }

    #[test]
    fn requires_canonical_file_order_and_entry() {
        let index = BundleFile::new("index.html", 1, DIGEST).expect("index is valid");
        let script = BundleFile::new("presentation.js", 1, DIGEST).expect("script is valid");

        assert_eq!(
            BundleManifest::new(
                PresentationTemporalCapability::Sequential,
                PresentationVisualCapability::BrowserComposite,
                DIGEST,
                vec![script.clone(), index.clone()],
            ),
            Err(InvalidBundleManifest::UnorderedFiles),
        );
        assert_eq!(
            BundleManifest::new(
                PresentationTemporalCapability::Sequential,
                PresentationVisualCapability::BrowserComposite,
                DIGEST,
                vec![script],
            ),
            Err(InvalidBundleManifest::MissingEntryPoint),
        );
        assert!(
            BundleManifest::new(
                PresentationTemporalCapability::Sequential,
                PresentationVisualCapability::BrowserComposite,
                DIGEST,
                vec![index],
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_file_and_directory_path_collisions() {
        let index = BundleFile::new("index.html", 1, DIGEST).expect("index is valid");
        let sibling =
            BundleFile::new("index.html-other", 1, DIGEST).expect("the sibling path is valid");
        let descendant =
            BundleFile::new("index.html/child", 1, DIGEST).expect("the child path is valid");

        assert_eq!(
            BundleManifest::new(
                PresentationTemporalCapability::Sequential,
                PresentationVisualCapability::BrowserComposite,
                DIGEST,
                vec![index, sibling, descendant],
            ),
            Err(InvalidBundleManifest::FilePathConflict),
        );
    }

    #[test]
    fn rejects_unknown_versions_and_incomplete_capabilities() {
        for source in [
            format!(
                r#"{{"version":2,"bundleId":"{DIGEST}","entryPoint":"index.html","temporalCapability":"randomAccess","visualCapability":"browserComposite","files":[{{"bytes":1,"path":"index.html","sha256":"{DIGEST}"}}]}}"#,
            ),
            format!(
                r#"{{"version":1,"bundleId":"{DIGEST}","entryPoint":"index.html","files":[{{"bytes":1,"path":"index.html","sha256":"{DIGEST}"}}]}}"#,
            ),
            format!(
                r#"{{"version":1,"bundleId":"{DIGEST}","entryPoint":"index.html","temporalCapability":"sequential","files":[{{"bytes":1,"path":"index.html","sha256":"{DIGEST}"}}]}}"#,
            ),
        ] {
            assert!(serde_json::from_str::<BundleManifest>(&source).is_err());
        }
    }

    #[test]
    fn current_identity_records_both_capabilities() {
        let file = BundleFile::new("index.html", 1, DIGEST).expect("index is valid");
        let manifest = BundleManifest::new(
            PresentationTemporalCapability::RandomAccess,
            PresentationVisualCapability::SeparableOverlay,
            DIGEST,
            vec![file],
        )
        .expect("the fixture manifest is canonical");

        assert_eq!(manifest.version(), super::BundleVersion::CURRENT);
        assert_eq!(
            serde_json::to_string(&manifest.identity()).expect("identity serializes"),
            format!(
                r#"{{"version":1,"entryPoint":"index.html","temporalCapability":"randomAccess","visualCapability":"separableOverlay","files":[{{"bytes":1,"path":"index.html","sha256":"{DIGEST}"}}]}}"#,
            ),
        );
    }
}
