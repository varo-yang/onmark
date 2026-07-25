//! Fixed product-version ownership for release pull requests.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use semver::Version;
use serde_json::Value;

const CARGO_MANIFEST: &str = "Cargo.toml";
// Package additions must make their fixed-version participation explicit here.
const PACKAGE_MANIFESTS: [&str; 5] = [
    "packages/authoring/package.json",
    "packages/bundler/package.json",
    "packages/launcher/package.json",
    "packages/motion-gsap/package.json",
    "packages/runtime/package.json",
];
const WORKSPACE_PACKAGE_SECTION: &str = "[workspace.package]";

pub(super) fn prepare(repository: &Path, requested: &str) -> Result<(), VersionError> {
    let requested = parse_version(requested)?;
    let current = verified_product_version(repository, None)?;
    if requested <= current {
        return Err(VersionError::NotIncreasing { current, requested });
    }

    let files = prepare_manifests(repository, &current, &requested)?;
    for file in files {
        fs::write(&file.path, file.contents)
            .map_err(|source| VersionError::io("write release manifest", &file.path, source))?;
    }

    refresh_cargo_lock(repository)?;
    verified_product_version(repository, Some(&requested)).map(drop)
}

pub(super) fn verify(repository: &Path, expected: Option<&str>) -> Result<(), VersionError> {
    let expected = expected.map(parse_version).transpose()?;
    verified_product_version(repository, expected.as_ref()).map(drop)
}

fn verified_product_version(
    repository: &Path,
    expected: Option<&Version>,
) -> Result<Version, VersionError> {
    let product = read_product_version(repository)?;
    if let Some(expected) = expected {
        require_version(CARGO_MANIFEST, &product, expected)?;
    }

    for manifest in PACKAGE_MANIFESTS {
        let version = read_package_version(repository, manifest)?;
        require_version(manifest, &version, &product)?;
    }
    Ok(product)
}

// ── Manifest preparation

fn prepare_manifests(
    repository: &Path,
    current: &Version,
    requested: &Version,
) -> Result<Vec<PreparedFile>, VersionError> {
    let mut files = Vec::with_capacity(PACKAGE_MANIFESTS.len() + 1);
    let cargo = repository.join(CARGO_MANIFEST);
    let contents = read(&cargo, "read workspace manifest")?;
    files.push(PreparedFile {
        path: cargo,
        contents: replace_workspace_version(&contents, current, requested)?,
    });

    for manifest in PACKAGE_MANIFESTS {
        let path = repository.join(manifest);
        let contents = read(&path, "read package manifest")?;
        files.push(PreparedFile {
            path,
            contents: replace_package_version(manifest, &contents, current, requested)?,
        });
    }
    Ok(files)
}

// ── Manifest parsing

fn read_product_version(repository: &Path) -> Result<Version, VersionError> {
    let path = repository.join(CARGO_MANIFEST);
    let contents = read(&path, "read workspace manifest")?;
    let value =
        workspace_version(&contents).ok_or_else(|| VersionError::MissingVersion(path.clone()))?;
    parse_version(value)
}

fn read_package_version(repository: &Path, manifest: &str) -> Result<Version, VersionError> {
    let path = repository.join(manifest);
    let contents = read(&path, "read package manifest")?;
    let document: Value = serde_json::from_str(&contents).map_err(|source| VersionError::Json {
        path: path.clone(),
        source,
    })?;
    let value = document
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| VersionError::MissingVersion(path))?;
    parse_version(value)
}

fn workspace_version(contents: &str) -> Option<&str> {
    let mut in_workspace_package = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_workspace_package = line == WORKSPACE_PACKAGE_SECTION;
            continue;
        }
        if !in_workspace_package {
            continue;
        }
        if let Some(value) = quoted_assignment(line, "version") {
            return Some(value);
        }
    }
    None
}

fn quoted_assignment<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.strip_prefix(name)?
        .trim_start()
        .strip_prefix('=')?
        .trim()
        .strip_prefix('"')?
        .strip_suffix('"')
}

fn replace_workspace_version(
    contents: &str,
    current: &Version,
    requested: &Version,
) -> Result<String, VersionError> {
    let current = format!("version = \"{current}\"");
    let requested = format!("version = \"{requested}\"");
    replace_once(CARGO_MANIFEST, contents, &current, &requested)
}

fn replace_package_version(
    manifest: &str,
    contents: &str,
    current: &Version,
    requested: &Version,
) -> Result<String, VersionError> {
    let current = format!("\"version\": \"{current}\"");
    let requested = format!("\"version\": \"{requested}\"");
    replace_once(manifest, contents, &current, &requested)
}

fn replace_once(
    manifest: &str,
    contents: &str,
    current: &str,
    requested: &str,
) -> Result<String, VersionError> {
    if contents.matches(current).count() != 1 {
        return Err(VersionError::Replacement {
            path: PathBuf::from(manifest),
        });
    }
    Ok(contents.replacen(current, requested, 1))
}

fn parse_version(value: &str) -> Result<Version, VersionError> {
    Version::parse(value).map_err(|source| VersionError::InvalidVersion {
        value: value.into(),
        source,
    })
}

fn require_version(
    manifest: &str,
    actual: &Version,
    expected: &Version,
) -> Result<(), VersionError> {
    if actual == expected {
        return Ok(());
    }
    Err(VersionError::Mismatch {
        path: PathBuf::from(manifest),
        actual: actual.clone(),
        expected: expected.clone(),
    })
}

// ── Filesystem and Cargo boundary

fn refresh_cargo_lock(repository: &Path) -> Result<(), VersionError> {
    let status = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(repository)
        .stdout(Stdio::null())
        .status()
        .map_err(|source| VersionError::io("start Cargo metadata", repository, source))?;
    if status.success() {
        return Ok(());
    }
    Err(VersionError::Cargo(status))
}

fn read(path: &Path, operation: &'static str) -> Result<String, VersionError> {
    fs::read_to_string(path).map_err(|source| VersionError::io(operation, path, source))
}

struct PreparedFile {
    path: PathBuf,
    contents: String,
}

// ── Failures

#[derive(Debug)]
pub(crate) enum VersionError {
    Cargo(ExitStatus),
    InvalidVersion {
        value: Box<str>,
        source: semver::Error,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Mismatch {
        path: PathBuf,
        actual: Version,
        expected: Version,
    },
    MissingVersion(PathBuf),
    Replacement {
        path: PathBuf,
    },
    NotIncreasing {
        current: Version,
        requested: Version,
    },
}

impl VersionError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for VersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cargo(status) => write!(formatter, "Cargo metadata exited with {status}"),
            Self::InvalidVersion { value, .. } => {
                write!(formatter, "invalid release version {value}")
            }
            Self::Io {
                operation, path, ..
            } => write!(formatter, "cannot {operation} {}", path.display()),
            Self::Json { path, .. } => {
                write!(
                    formatter,
                    "cannot parse package manifest {}",
                    path.display()
                )
            }
            Self::Mismatch {
                path,
                actual,
                expected,
            } => write!(
                formatter,
                "{} has version {actual}; expected {expected}",
                path.display()
            ),
            Self::MissingVersion(path) => {
                write!(formatter, "{} has no release version", path.display())
            }
            Self::Replacement { path } => {
                write!(
                    formatter,
                    "{} does not contain one canonical version field",
                    path.display()
                )
            }
            Self::NotIncreasing { current, requested } => write!(
                formatter,
                "release version {requested} does not advance current version {current}"
            ),
        }
    }
}

impl Error for VersionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidVersion { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Cargo(_)
            | Self::Mismatch { .. }
            | Self::MissingVersion(_)
            | Self::NotIncreasing { .. }
            | Self::Replacement { .. } => None,
        }
    }
}

// ── Tests

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn reads_only_the_workspace_package_version() {
        let manifest = r#"
[workspace]
resolver = "3"

[workspace.package]
version = "1.2.3-rc.1"
rust-version = "1.97"

[workspace.dependencies]
version = "9.9.9"
"#;

        assert_eq!(workspace_version(manifest), Some("1.2.3-rc.1"));
    }

    #[test]
    fn replaces_one_canonical_version_without_reformatting() {
        let contents = "{\n  \"name\": \"onmark\",\n  \"version\": \"1.2.3\"\n}\n";
        let updated = replace_package_version(
            "package.json",
            contents,
            &Version::new(1, 2, 3),
            &Version::new(2, 0, 0),
        )
        .expect("one canonical version is replaceable");

        assert_eq!(
            updated,
            "{\n  \"name\": \"onmark\",\n  \"version\": \"2.0.0\"\n}\n"
        );
    }

    #[test]
    fn rejects_ambiguous_version_fields() {
        let contents = "\"version\": \"1.2.3\"\n\"version\": \"1.2.3\"\n";
        let result = replace_package_version(
            "package.json",
            contents,
            &Version::new(1, 2, 3),
            &Version::new(2, 0, 0),
        );

        assert!(matches!(result, Err(VersionError::Replacement { .. })));
    }

    #[test]
    fn verifies_every_fixed_product_manifest() {
        let repository = repository_fixture("1.2.3");

        verify(repository.path(), Some("1.2.3"))
            .expect("matching fixed product versions are valid");
    }

    #[test]
    fn identifies_the_mismatched_manifest() {
        let repository = repository_fixture("1.2.3");
        let path = repository.path().join(PACKAGE_MANIFESTS[2]);
        fs::write(&path, package_manifest("1.2.4"))
            .expect("the fixture package manifest is replaceable");

        let result = verify(repository.path(), None);

        assert!(matches!(
            result,
            Err(VersionError::Mismatch { path, .. })
                if path == Path::new(PACKAGE_MANIFESTS[2])
        ));
    }

    fn repository_fixture(version: &str) -> TempDir {
        let repository = tempfile::tempdir().expect("the fixture repository can be created");
        fs::write(
            repository.path().join(CARGO_MANIFEST),
            format!("{WORKSPACE_PACKAGE_SECTION}\nversion = \"{version}\"\n"),
        )
        .expect("the fixture workspace manifest can be written");

        for manifest in PACKAGE_MANIFESTS {
            let path = repository.path().join(manifest);
            fs::create_dir_all(path.parent().expect("a package manifest has a parent"))
                .expect("the fixture package directory can be created");
            fs::write(path, package_manifest(version))
                .expect("the fixture package manifest can be written");
        }
        repository
    }

    fn package_manifest(version: &str) -> String {
        format!("{{\n  \"version\": \"{version}\"\n}}\n")
    }
}
