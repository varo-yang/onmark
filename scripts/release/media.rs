//! Admission of the repository-owned FFmpeg build into a platform sidecar.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use super::artifact::hash_file;
use super::error::PackageError;
use super::target::{ExecutableRole, ReleaseTarget};

const MAX_BUILD_RECORD_BYTES: u64 = 128 * 1024;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const SOURCE_MANIFEST: &str = "scripts/release/media-sources.json";

pub(super) struct MediaBundle {
    manifest: MediaSources,
    files: Vec<MediaFile>,
}

impl MediaBundle {
    pub(super) fn admit(
        repository: &Path,
        root: &Path,
        target: ReleaseTarget,
    ) -> Result<Self, PackageError> {
        let canonical_manifest = repository.join(SOURCE_MANIFEST);
        let supplied_manifest = root.join("media-sources.json");
        require_regular_file(
            &supplied_manifest,
            MAX_BUILD_RECORD_BYTES,
            "media source manifest is not a bounded regular file",
        )?;
        require_equal_files(
            &canonical_manifest,
            &supplied_manifest,
            "media source manifest differs from the repository contract",
        )?;
        let canonical_build = repository.join("scripts/release/build-media.sh");
        let supplied_build = root.join("sources/build-media.sh");
        require_regular_file(
            &supplied_build,
            MAX_BUILD_RECORD_BYTES,
            "media build script is not a bounded regular file",
        )?;
        require_equal_files(
            &canonical_build,
            &supplied_build,
            "media build script differs from the repository contract",
        )?;

        let contents = fs::read_to_string(&canonical_manifest).map_err(|source| {
            PackageError::io("read media source manifest", &canonical_manifest, source)
        })?;
        let manifest: MediaSources = serde_json::from_str(&contents)?;
        manifest.validate(root)?;

        target.validate_executable(
            &root
                .join("bin")
                .join(target.executable_name(ExecutableRole::Ffmpeg)),
            ExecutableRole::Ffmpeg,
        )?;
        target.validate_executable(
            &root
                .join("bin")
                .join(target.executable_name(ExecutableRole::Ffprobe)),
            ExecutableRole::Ffprobe,
        )?;
        require_regular_file(
            &root.join("build.txt"),
            MAX_BUILD_RECORD_BYTES,
            "media build record is not a bounded regular file",
        )?;
        for license in ["FFmpeg-GPLv2.txt", "x264-GPLv2.txt", "zlib.txt"] {
            require_regular_file(
                &root.join("licenses").join(license),
                MAX_LICENSE_BYTES,
                "media license is not a bounded regular file",
            )?;
        }

        let files = media_files(root, &manifest, target);
        Ok(Self { manifest, files })
    }

    pub(super) fn ffmpeg_version(&self) -> &str {
        &self.manifest.ffmpeg.version
    }

    pub(super) fn x264_revision(&self) -> &str {
        &self.manifest.x264.revision
    }

    pub(super) fn files(&self) -> &[MediaFile] {
        &self.files
    }
}

pub(super) struct MediaFile {
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) kind: MediaFileKind,
}

impl MediaFile {
    const fn new(source: PathBuf, destination: PathBuf, kind: MediaFileKind) -> Self {
        Self {
            source,
            destination,
            kind,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum MediaFileKind {
    Data,
    Executable,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MediaSources {
    schema_version: u8,
    ffmpeg: VersionedSource,
    x264: RevisionSource,
    zlib: VersionedSource,
}

impl MediaSources {
    fn validate(&self, root: &Path) -> Result<(), PackageError> {
        if self.schema_version != 1 {
            return Err(PackageError::invalid_input(
                &root.join("media-sources.json"),
                "unsupported media source manifest",
            ));
        }
        for source in self.sources() {
            source.validate(&root.join("sources"))?;
        }
        Ok(())
    }

    fn sources(&self) -> [&Source; 3] {
        [&self.ffmpeg.source, &self.x264.source, &self.zlib.source]
    }
}

#[derive(Deserialize)]
struct VersionedSource {
    version: Box<str>,
    #[serde(flatten)]
    source: Source,
}

#[derive(Deserialize)]
struct RevisionSource {
    revision: Box<str>,
    #[serde(flatten)]
    source: Source,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    name: Box<str>,
    url: Box<str>,
    bytes: u64,
    sha256: Box<str>,
}

impl Source {
    fn validate(&self, directory: &Path) -> Result<(), PackageError> {
        if !portable_filename(&self.name)
            || !self.url.starts_with("https://")
            || self.bytes == 0
            || self.bytes > MAX_SOURCE_BYTES
            || !is_sha256(&self.sha256)
        {
            return Err(PackageError::invalid_input(
                directory,
                "media source manifest contains an invalid artifact",
            ));
        }

        let path = directory.join(&*self.name);
        let bytes = require_regular_file(
            &path,
            MAX_SOURCE_BYTES,
            "media source is not a bounded regular file",
        )?;
        if bytes != self.bytes || hash_file(&path)? != self.sha256.as_ref() {
            return Err(PackageError::invalid_input(
                &path,
                "media source does not match its admitted identity",
            ));
        }
        Ok(())
    }
}

fn require_equal_files(
    expected: &Path,
    actual: &Path,
    reason: &'static str,
) -> Result<(), PackageError> {
    let expected_bytes = fs::read(expected)
        .map_err(|source| PackageError::io("read canonical release contract", expected, source))?;
    let actual_bytes = fs::read(actual)
        .map_err(|source| PackageError::io("read supplied release input", actual, source))?;
    if expected_bytes != actual_bytes {
        return Err(PackageError::invalid_input(actual, reason));
    }
    Ok(())
}

fn require_regular_file(
    path: &Path,
    limit: u64,
    reason: &'static str,
) -> Result<u64, PackageError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| PackageError::io("inspect media release input", path, source))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(PackageError::invalid_input(path, reason));
    }
    Ok(metadata.len())
}

fn portable_filename(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn media_files(root: &Path, manifest: &MediaSources, target: ReleaseTarget) -> Vec<MediaFile> {
    let mut files = Vec::with_capacity(11);
    for role in [ExecutableRole::Ffmpeg, ExecutableRole::Ffprobe] {
        let relative = PathBuf::from("bin").join(target.executable_name(role));
        files.push(MediaFile::new(
            root.join(&relative),
            relative,
            MediaFileKind::Executable,
        ));
    }
    for (source, destination) in [
        ("build.txt", "media-build.txt"),
        ("media-sources.json", "media-sources.json"),
    ] {
        files.push(MediaFile::new(
            root.join(source),
            PathBuf::from(destination),
            MediaFileKind::Data,
        ));
    }
    for license in ["FFmpeg-GPLv2.txt", "x264-GPLv2.txt", "zlib.txt"] {
        let relative = PathBuf::from("licenses").join(license);
        files.push(MediaFile::new(
            root.join(&relative),
            relative,
            MediaFileKind::Data,
        ));
    }
    files.push(MediaFile::new(
        root.join("sources/build-media.sh"),
        PathBuf::from("sources/build-media.sh"),
        MediaFileKind::Data,
    ));
    for source in manifest.sources() {
        let relative = PathBuf::from("sources").join(&*source.name);
        files.push(MediaFile::new(
            root.join(&relative),
            relative,
            MediaFileKind::Data,
        ));
    }
    files
}
