//! Canonical file identity and retained-size accounting for release artifacts.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::error::PackageError;

pub(super) const MAX_SOURCE_REVISION_BYTES: usize = 256;

pub(super) fn parse_source_revision(value: String, flag: &str) -> Result<Box<str>, PackageError> {
    if value.trim().is_empty()
        || value.len() > MAX_SOURCE_REVISION_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(PackageError::InvalidOptions(
            format!(
                "{flag} must be non-empty single-line text of at most \
                 {MAX_SOURCE_REVISION_BYTES} bytes"
            )
            .into_boxed_str(),
        ));
    }
    Ok(value.into_boxed_str())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Artifact {
    path: String,
    bytes: u64,
    sha256: String,
}

impl Artifact {
    pub(super) fn inspect(root: &Path, relative_path: &Path) -> Result<Self, PackageError> {
        let path = root.join(relative_path);
        let bytes = fs::metadata(&path)
            .map_err(|source| PackageError::io("inspect release artifact", &path, source))?
            .len();
        let sha256 = hash_file(&path)?;
        Ok(Self {
            path: portable_path(relative_path),
            bytes,
            sha256,
        })
    }
}

pub(super) fn hash_file(path: &Path) -> Result<String, PackageError> {
    let mut file = File::open(path)
        .map_err(|source| PackageError::io("open release artifact", path, source))?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)
        .map_err(|source| PackageError::io("hash release artifact", path, source))?;
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn enforce_size(
    root: &Path,
    relative_paths: &[PathBuf],
    limit: u64,
) -> Result<(), PackageError> {
    let actual = relative_paths.iter().try_fold(0_u64, |total, relative| {
        let path = root.join(relative);
        fs::metadata(&path)
            .map(|metadata| total.saturating_add(metadata.len()))
            .map_err(|source| PackageError::io("inspect release artifact", &path, source))
    })?;
    if actual > limit {
        return Err(PackageError::PackageLimit { actual, limit });
    }
    Ok(())
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::parse_source_revision;

    #[test]
    fn bounds_release_source_revision_text() {
        assert!(parse_source_revision("abcdef".into(), "--source-revision").is_ok());
        assert!(parse_source_revision("".into(), "--source-revision").is_err());
        assert!(parse_source_revision("x".repeat(257), "--source-revision").is_err());
        assert!(parse_source_revision("line\nbreak".into(), "--source-revision").is_err());
    }
}
