//! Deployment-owned materialization of a pinned browser payload.
//!
//! The deployment may provide an expanded browser or one compact archive. The
//! archive form is verified and expanded sequentially into the worker's private
//! `/tmp` filesystem before the renderer launches a process. Preparation is
//! lazy so Lambda begins Runtime API polling before it touches the large
//! payload, while warm invocations retain the same private installation.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use onmark_render::{BrowserLaunchPolicy, BrowserLimits, Ffmpeg, FrameCaptureExecutor};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tokio::task::JoinError;

use crate::{
    BROWSER_ARCHIVE_EXECUTABLE, BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES, BROWSER_ARCHIVE_MAX_ENTRIES,
    BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
};

/// A validated digest for one immutable compressed browser payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserArchiveDigest(Box<str>);

impl BrowserArchiveDigest {
    pub(crate) fn parse(value: &str) -> Result<Self, InvalidBrowserArchiveDigest> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(InvalidBrowserArchiveDigest);
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(InvalidBrowserArchiveDigest);
        }
        Ok(Self(value.into()))
    }
}

impl fmt::Display for BrowserArchiveDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The deployment-owned representation from which a browser is installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BrowserPackage {
    Expanded(PathBuf),
    Archive(BrowserArchive),
}

impl BrowserPackage {
    pub(crate) fn expanded(path: PathBuf) -> Self {
        Self::Expanded(path)
    }

    pub(crate) fn archive(path: PathBuf, digest: BrowserArchiveDigest) -> Self {
        Self::Archive(BrowserArchive::new(path, digest))
    }

    pub(crate) async fn materialize(
        &self,
    ) -> Result<BrowserInstallation, BrowserInstallationError> {
        match self {
            Self::Expanded(executable) => BrowserInstallation::expanded(executable),
            Self::Archive(archive) => {
                let archive = archive.clone();
                tokio::task::spawn_blocking(move || archive.materialize())
                    .await
                    .map_err(BrowserInstallationError::Task)?
            }
        }
    }
}

/// Lazily prepared browser state shared by every invocation in one worker.
///
/// Construction is deliberately free of browser I/O. Lambda can therefore
/// start polling the Runtime API before a fresh worker verifies and expands
/// its pinned browser payload. The first capture pays that bounded cost inside
/// the invocation deadline; warm captures reuse the same private installation.
#[derive(Debug)]
pub(crate) struct BrowserRuntime {
    package: BrowserPackage,
    limits: BrowserLimits,
    installation: OnceCell<BrowserInstallation>,
    ffmpeg: Ffmpeg,
}

impl BrowserRuntime {
    pub(crate) const fn new(
        package: BrowserPackage,
        limits: BrowserLimits,
        ffmpeg: Ffmpeg,
    ) -> Self {
        Self {
            package,
            limits,
            installation: OnceCell::const_new(),
            ffmpeg,
        }
    }

    pub(crate) async fn executor(&self) -> Result<FrameCaptureExecutor, BrowserInstallationError> {
        let installation = self
            .installation
            .get_or_try_init(|| self.package.materialize())
            .await?;
        Ok(FrameCaptureExecutor::new(
            installation.executable().to_owned(),
            BrowserLaunchPolicy::isolated_worker(),
            self.limits,
            self.ffmpeg.clone(),
        ))
    }

    #[cfg(test)]
    fn is_ready(&self) -> bool {
        self.installation.initialized()
    }
}

/// One immutable zstd-compressed tar payload owned by the Lambda deployment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BrowserArchive {
    path: PathBuf,
    digest: BrowserArchiveDigest,
}

impl BrowserArchive {
    pub(crate) fn new(path: PathBuf, digest: BrowserArchiveDigest) -> Self {
        Self { path, digest }
    }

    pub(crate) fn materialize(&self) -> Result<BrowserInstallation, BrowserInstallationError> {
        let root =
            TempDir::new_in("/tmp").map_err(|source| self.io("create staging root", source))?;
        let actual = self.unpack_into(root.path())?;
        if actual != self.digest.0.as_ref() {
            return Err(BrowserInstallationError::DigestMismatch {
                path: self.path.clone(),
                expected: self.digest.clone(),
                actual: actual.into_boxed_str(),
            });
        }
        self.write_font_configuration(root.path())?;

        let executable = root.path().join(BROWSER_ARCHIVE_EXECUTABLE);
        require_executable(&executable)?;
        Ok(BrowserInstallation {
            executable,
            _root: Some(root),
        })
    }

    fn open(&self) -> Result<File, BrowserInstallationError> {
        let file = File::open(&self.path).map_err(|source| self.io("open archive", source))?;
        let archive_bytes = file
            .metadata()
            .map_err(|source| self.io("inspect archive", source))?
            .len();
        if archive_bytes > BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES {
            return Err(BrowserInstallationError::ArchiveLimit {
                path: self.path.clone(),
                limit: BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES,
            });
        }

        Ok(file)
    }

    fn unpack_into(&self, root: &Path) -> Result<String, BrowserInstallationError> {
        let reader = DigestReader::new(self.open()?);
        let decoder =
            zstd::Decoder::new(reader).map_err(|source| self.io("decode archive", source))?;
        let mut archive = tar::Archive::new(decoder);
        let entries = archive
            .entries()
            .map_err(|source| self.io("read archive", source))?;
        let mut budget = ArchiveBudget::default();

        for entry in entries {
            let mut entry = entry.map_err(|source| self.io("read archive entry", source))?;
            let path = entry
                .path()
                .map_err(|source| self.io("read archive entry path", source))?
                .into_owned();
            let size = entry
                .header()
                .size()
                .map_err(|source| self.io("read archive entry size", source))?;
            budget.accept(&path, entry.header().entry_type(), size)?;
            let unpacked = entry
                .unpack_in(root)
                .map_err(|source| self.io("unpack archive entry", source))?;
            if !unpacked {
                return Err(BrowserInstallationError::InvalidEntry(path));
            }
        }
        let mut decoder = archive.into_inner();
        io::copy(&mut decoder, &mut io::sink())
            .map_err(|source| self.io("finish archive", source))?;
        let reader = decoder.finish().into_inner();
        Ok(reader.finish())
    }

    fn write_font_configuration(&self, root: &Path) -> Result<(), BrowserInstallationError> {
        let fonts = root.join("fonts");
        if !fonts.is_dir() {
            return Ok(());
        }
        let cache = root.join("font-cache");
        fs::create_dir(&cache).map_err(|source| self.io("create font cache directory", source))?;
        let configuration = format!(
            "<?xml version=\"1.0\"?>\n<fontconfig>\n  <dir>{}</dir>\n  <cachedir>{}</cachedir>\n</fontconfig>\n",
            xml_text(&fonts),
            xml_text(&cache),
        );
        fs::write(root.join("fonts.conf"), configuration)
            .map_err(|source| self.io("write font configuration", source))
    }

    fn io(&self, operation: &'static str, source: io::Error) -> BrowserInstallationError {
        BrowserInstallationError::Io {
            operation,
            path: self.path.clone(),
            source,
        }
    }
}

/// One browser executable whose private installation outlives every session.
#[derive(Debug)]
pub(crate) struct BrowserInstallation {
    executable: PathBuf,
    _root: Option<TempDir>,
}

impl BrowserInstallation {
    fn expanded(executable: &Path) -> Result<Self, BrowserInstallationError> {
        require_executable(executable)?;
        Ok(Self {
            executable: executable.to_owned(),
            _root: None,
        })
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }
}

#[derive(Default)]
struct ArchiveBudget {
    paths: BTreeSet<PathBuf>,
    expanded_bytes: u64,
}

impl ArchiveBudget {
    fn accept(
        &mut self,
        path: &Path,
        entry_type: tar::EntryType,
        size: u64,
    ) -> Result<(), BrowserInstallationError> {
        if !is_relative_archive_path(path)
            || (!entry_type.is_file() && !entry_type.is_dir())
            || !self.paths.insert(path.to_owned())
        {
            return Err(BrowserInstallationError::InvalidEntry(path.to_owned()));
        }
        if self.paths.len() > BROWSER_ARCHIVE_MAX_ENTRIES {
            return Err(BrowserInstallationError::EntryLimit {
                actual: self.paths.len(),
                limit: BROWSER_ARCHIVE_MAX_ENTRIES,
            });
        }

        self.expanded_bytes = self.expanded_bytes.checked_add(size).ok_or(
            BrowserInstallationError::ExpandedLimit {
                actual: u64::MAX,
                limit: BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
            },
        )?;
        if self.expanded_bytes > BROWSER_ARCHIVE_MAX_EXPANDED_BYTES {
            return Err(BrowserInstallationError::ExpandedLimit {
                actual: self.expanded_bytes,
                limit: BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
            });
        }
        Ok(())
    }
}

fn is_relative_archive_path(path: &Path) -> bool {
    let mut components = path.components();
    let Some(Component::Normal(_)) = components.next() else {
        return false;
    };
    components.all(|component| matches!(component, Component::Normal(_)))
}

fn xml_text(path: &Path) -> String {
    // Archive installations always live below the literal UTF-8 path `/tmp`;
    // replacing an unrepresentable path would make fontconfig inspect a
    // different directory from the one whose bytes were verified.
    path.to_str()
        .expect("private browser installation paths remain valid UTF-8")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn require_executable(path: &Path) -> Result<(), BrowserInstallationError> {
    let metadata = path
        .metadata()
        .map_err(|source| BrowserInstallationError::Io {
            operation: "inspect browser executable",
            path: path.to_owned(),
            source,
        })?;
    if !metadata.is_file() || !has_executable_bit(&metadata) {
        return Err(BrowserInstallationError::InvalidExecutable(path.to_owned()));
    }
    Ok(())
}

#[cfg(unix)]
fn has_executable_bit(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_executable_bit(_metadata: &std::fs::Metadata) -> bool {
    true
}

struct DigestReader<R> {
    inner: R,
    digest: Sha256,
}

impl<R> DigestReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            digest: Sha256::new(),
        }
    }

    fn finish(self) -> String {
        format_digest(&self.digest.finalize())
    }
}

impl<R: Read> Read for DigestReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.digest.update(&buffer[..count]);
        Ok(count)
    }
}

fn format_digest(bytes: &[u8]) -> String {
    let mut digest = String::with_capacity("sha256:".len() + bytes.len() * 2);
    digest.push_str("sha256:");
    for byte in bytes {
        write!(&mut digest, "{byte:02x}").expect("writing to a String cannot fail");
    }
    digest
}

/// Reason a configured browser-archive digest is not canonical SHA-256.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidBrowserArchiveDigest;

impl fmt::Display for InvalidBrowserArchiveDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("browser archive digest must be canonical sha256:<lowercase-hex>")
    }
}

impl Error for InvalidBrowserArchiveDigest {}

/// Reason a pinned browser payload cannot become a private installation.
#[derive(Debug)]
pub(crate) enum BrowserInstallationError {
    Task(JoinError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    ArchiveLimit {
        path: PathBuf,
        limit: u64,
    },
    DigestMismatch {
        path: PathBuf,
        expected: BrowserArchiveDigest,
        actual: Box<str>,
    },
    InvalidEntry(PathBuf),
    EntryLimit {
        actual: usize,
        limit: usize,
    },
    ExpandedLimit {
        actual: u64,
        limit: u64,
    },
    InvalidExecutable(PathBuf),
}

impl fmt::Display for BrowserInstallationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Task(_) => formatter.write_str("browser installation task did not complete"),
            Self::Io {
                operation, path, ..
            } => write!(formatter, "failed to {operation} {}", path.display()),
            Self::ArchiveLimit { path, limit } => write!(
                formatter,
                "browser archive {} exceeds its {limit}-byte compressed limit",
                path.display(),
            ),
            Self::DigestMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "browser archive {} has digest {actual}, expected {expected}",
                path.display(),
            ),
            Self::InvalidEntry(path) => write!(
                formatter,
                "browser archive contains invalid entry {}",
                path.display(),
            ),
            Self::EntryLimit { actual, limit } => write!(
                formatter,
                "browser archive contains {actual} entries, exceeding its {limit}-entry limit",
            ),
            Self::ExpandedLimit { actual, limit } => write!(
                formatter,
                "browser archive expands to {actual} bytes, exceeding its {limit}-byte limit",
            ),
            Self::InvalidExecutable(path) => write!(
                formatter,
                "browser archive does not provide an executable {}",
                path.display(),
            ),
        }
    }
}

impl Error for BrowserInstallationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Task(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::ArchiveLimit { .. }
            | Self::DigestMismatch { .. }
            | Self::InvalidEntry(_)
            | Self::EntryLimit { .. }
            | Self::ExpandedLimit { .. }
            | Self::InvalidExecutable(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::Path;
    use std::time::Duration;

    use onmark_render::{BrowserLimits, EncodeLimits, Ffmpeg};
    use sha2::{Digest as _, Sha256};
    use tar::{Builder, Header};
    use tempfile::TempDir;

    use super::{
        ArchiveBudget, BrowserArchive, BrowserArchiveDigest, BrowserInstallationError,
        BrowserPackage, BrowserRuntime, InvalidBrowserArchiveDigest, format_digest,
    };
    use crate::BROWSER_ARCHIVE_MAX_EXPANDED_BYTES;

    #[tokio::test]
    async fn defers_browser_io_until_the_runtime_is_requested() {
        let fixture = TempDir::new().expect("the fixture directory is writable");
        let missing = fixture.path().join("missing-browser");
        let limits = BrowserLimits::new(Duration::from_secs(1), 1024)
            .expect("the fixture browser limits are bounded");

        let runtime =
            BrowserRuntime::new(BrowserPackage::expanded(missing), limits, fixture_ffmpeg());

        assert!(!runtime.is_ready());
        assert!(matches!(
            runtime.executor().await,
            Err(BrowserInstallationError::Io { .. }),
        ));
        assert!(!runtime.is_ready());
    }

    #[tokio::test]
    async fn reuses_one_verified_archive_installation() {
        let fixture = TempDir::new().expect("the fixture directory is writable");
        let archive = fixture.path().join("browser.tar.zst");
        let bytes = browser_archive(&[("chrome-headless-shell", b"browser", 0o755)]);
        fs::write(&archive, &bytes).expect("the fixture archive is writable");
        let digest = BrowserArchiveDigest::parse(&format_digest(&Sha256::digest(&bytes)))
            .expect("the fixture digest is valid");
        let limits = BrowserLimits::new(Duration::from_secs(1), 1024)
            .expect("the fixture browser limits are bounded");
        let runtime = BrowserRuntime::new(
            BrowserPackage::archive(archive.clone(), digest),
            limits,
            fixture_ffmpeg(),
        );

        runtime
            .executor()
            .await
            .expect("the first request installs the browser");
        fs::remove_file(archive).expect("the source archive can disappear after installation");
        runtime
            .executor()
            .await
            .expect("the warm request reuses the retained installation");

        assert!(runtime.is_ready());
    }

    #[test]
    fn materializes_a_digest_verified_browser_archive() {
        let fixture = TempDir::new().expect("the fixture directory is writable");
        let archive = fixture.path().join("browser.tar.zst");
        let bytes = browser_archive(&[
            ("chrome-headless-shell", b"browser", 0o755),
            ("fonts/OpenSans.ttf", b"font", 0o644),
        ]);
        fs::write(&archive, &bytes).expect("the fixture archive is writable");
        let digest = BrowserArchiveDigest::parse(&format_digest(&Sha256::digest(&bytes)))
            .expect("the fixture digest is valid");

        let installation = BrowserArchive::new(archive, digest)
            .materialize()
            .expect("the browser archive materializes");

        assert_eq!(
            fs::read(installation.executable()).expect("the browser executable is readable"),
            b"browser",
        );
        let installation_root = installation
            .executable()
            .parent()
            .expect("the browser has an installation root");
        let font_configuration = fs::read_to_string(installation_root.join("fonts.conf"))
            .expect("the generated font configuration is readable");
        assert!(font_configuration.contains(&format!(
            "<dir>{}</dir>",
            installation_root.join("fonts").display(),
        )));
    }

    fn fixture_ffmpeg() -> Ffmpeg {
        let limits = EncodeLimits::new(Duration::from_secs(1), 1, 1, 1)
            .expect("the fixture encoder limits are bounded");
        Ffmpeg::new("ffmpeg", limits).expect("the fixture executable path is present")
    }

    #[test]
    fn rejects_an_archive_whose_bytes_do_not_match_its_identity() {
        let fixture = TempDir::new().expect("the fixture directory is writable");
        let archive = fixture.path().join("browser.tar.zst");
        let bytes = browser_archive(&[("chrome-headless-shell", b"browser", 0o755)]);
        fs::write(&archive, bytes).expect("the fixture archive is writable");
        let digest = BrowserArchiveDigest::parse(&format!("sha256:{}", "0".repeat(64)))
            .expect("the fixture digest is valid");

        let error = BrowserArchive::new(archive, digest)
            .materialize()
            .expect_err("modified archive bytes must be rejected");

        assert!(matches!(
            error,
            BrowserInstallationError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn rejects_noncanonical_archive_digests() {
        assert_eq!(
            BrowserArchiveDigest::parse(&format!("sha256:{}", "A".repeat(64))),
            Err(InvalidBrowserArchiveDigest),
        );
        assert_eq!(
            BrowserArchiveDigest::parse("sha256:1234"),
            Err(InvalidBrowserArchiveDigest),
        );
    }

    #[test]
    fn rejects_unsafe_duplicate_and_oversized_entries_before_unpacking() {
        let mut budget = ArchiveBudget::default();
        assert!(matches!(
            budget.accept(Path::new("../browser"), tar::EntryType::Regular, 1),
            Err(BrowserInstallationError::InvalidEntry(_)),
        ));
        assert!(matches!(
            budget.accept(Path::new("browser"), tar::EntryType::Symlink, 1),
            Err(BrowserInstallationError::InvalidEntry(_)),
        ));

        budget
            .accept(Path::new("browser"), tar::EntryType::Regular, 1)
            .expect("the first canonical entry fits");
        assert!(matches!(
            budget.accept(Path::new("browser"), tar::EntryType::Regular, 1),
            Err(BrowserInstallationError::InvalidEntry(_)),
        ));
        assert!(matches!(
            ArchiveBudget::default().accept(
                Path::new("browser"),
                tar::EntryType::Regular,
                BROWSER_ARCHIVE_MAX_EXPANDED_BYTES + 1,
            ),
            Err(BrowserInstallationError::ExpandedLimit { .. }),
        ));
    }

    #[test]
    fn requires_the_archive_to_provide_an_executable_browser() {
        let fixture = TempDir::new().expect("the fixture directory is writable");
        let archive = fixture.path().join("browser.tar.zst");
        let bytes = browser_archive(&[("chrome-headless-shell", b"browser", 0o644)]);
        fs::write(&archive, &bytes).expect("the fixture archive is writable");
        let digest = BrowserArchiveDigest::parse(&format_digest(&Sha256::digest(&bytes)))
            .expect("the fixture digest is valid");

        let error = BrowserArchive::new(archive, digest)
            .materialize()
            .expect_err("a non-executable browser must be rejected");

        assert!(matches!(
            error,
            BrowserInstallationError::InvalidExecutable(_)
        ));
    }

    fn browser_archive(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut tar = Builder::new(Vec::new());
        for (path, bytes, mode) in entries {
            let mut header = Header::new_gnu();
            header.set_size(u64::try_from(bytes.len()).expect("the fixture size fits u64"));
            header.set_mode(*mode);
            header.set_cksum();
            tar.append_data(&mut header, path, Cursor::new(bytes))
                .expect("the fixture entry appends");
        }
        let tar = tar.into_inner().expect("the fixture tar finishes");
        zstd::stream::encode_all(Cursor::new(tar), 3).expect("the fixture archive compresses")
    }
}
