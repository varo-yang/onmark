//! Reproducible packaging for the reviewed Lambda delivery shape.

mod archive;
mod error;
mod manifest;

use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use onmark_aws_lambda::BROWSER_ARCHIVE_EXECUTABLE;
use tempfile::Builder as TempBuilder;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use self::archive::BrowserArchive;
pub(crate) use self::error::PackageError;
use self::manifest::{Artifact, PackageManifest};

const ARCHIVE_NAME: &str = "browser.tar.zst";
const FFMPEG_NAME: &str = "ffmpeg";
const MANIFEST_NAME: &str = "manifest.json";
const PACKAGE_NAME: &str = "onmark-aws-lambda.zip";
const PACKAGE_TARGET: &str = "provided.al2023.arm64";
const CAPTURE_POLICY: &str = "isolated-worker-v1";
const MAX_UNZIPPED_PACKAGE_BYTES: u64 = 240 * 1024 * 1024;
const ELF_HEADER_BYTES: usize = 20;
const ELF_MACHINE_AARCH64: u16 = 183;

pub(crate) fn run(arguments: impl Iterator<Item = String>) -> Result<(), PackageError> {
    PackageBuilder::new(Options::parse(arguments)?).build()
}

#[derive(Debug)]
struct Options {
    bootstrap: PathBuf,
    browser_root: PathBuf,
    ffmpeg: PathBuf,
    output: PathBuf,
}

impl Options {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, PackageError> {
        let mut values = OptionValues::default();
        let mut arguments = arguments;

        while let Some(flag) = arguments.next() {
            let Some(value) = arguments.next() else {
                return Err(PackageError::InvalidOptions(
                    format!("{flag} requires a path").into_boxed_str(),
                ));
            };
            values.set(&flag, PathBuf::from(value))?;
        }

        values.finish()
    }
}

#[derive(Default)]
struct OptionValues {
    bootstrap: Option<PathBuf>,
    browser_root: Option<PathBuf>,
    ffmpeg: Option<PathBuf>,
    output: Option<PathBuf>,
}

impl OptionValues {
    fn set(&mut self, flag: &str, value: PathBuf) -> Result<(), PackageError> {
        let slot = match flag {
            "--bootstrap" => &mut self.bootstrap,
            "--browser-root" => &mut self.browser_root,
            "--ffmpeg" => &mut self.ffmpeg,
            "--output" => &mut self.output,
            _ => {
                return Err(PackageError::InvalidOptions(
                    format!("unknown option {flag}").into_boxed_str(),
                ));
            }
        };
        if slot.replace(value).is_some() {
            return Err(PackageError::InvalidOptions(
                format!("duplicate option {flag}").into_boxed_str(),
            ));
        }
        Ok(())
    }

    fn finish(self) -> Result<Options, PackageError> {
        Ok(Options {
            bootstrap: require_option(self.bootstrap, "--bootstrap")?,
            browser_root: require_option(self.browser_root, "--browser-root")?,
            ffmpeg: require_option(self.ffmpeg, "--ffmpeg")?,
            output: require_option(self.output, "--output")?,
        })
    }
}

fn require_option(value: Option<PathBuf>, flag: &'static str) -> Result<PathBuf, PackageError> {
    value.ok_or_else(|| PackageError::InvalidOptions(format!("missing {flag}").into_boxed_str()))
}

struct PackageBuilder {
    options: Options,
}

impl PackageBuilder {
    const fn new(options: Options) -> Self {
        Self { options }
    }

    fn build(self) -> Result<(), PackageError> {
        self.validate_inputs()?;
        let output_parent = output_parent(&self.options.output);
        fs::create_dir_all(output_parent)
            .map_err(|source| PackageError::io("create output parent", output_parent, source))?;
        if self.options.output.exists() {
            return Err(PackageError::OutputExists(self.options.output));
        }

        let staging = TempBuilder::new()
            .prefix(".onmark-lambda-package-")
            .tempdir_in(output_parent)
            .map_err(|source| {
                PackageError::io("create package staging directory", output_parent, source)
            })?;
        self.write_package(staging.path())?;

        fs::rename(staging.path(), &self.options.output).map_err(|source| {
            PackageError::io("publish package directory", &self.options.output, source)
        })?;
        let _published = staging.keep();
        Ok(())
    }

    fn validate_inputs(&self) -> Result<(), PackageError> {
        require_executable_file(&self.options.bootstrap, ExecutableRole::Bootstrap)?;
        require_directory(
            &self.options.browser_root,
            "browser root is not a directory",
        )?;
        require_executable_file(
            &self.options.browser_root.join(BROWSER_ARCHIVE_EXECUTABLE),
            ExecutableRole::Browser,
        )?;
        require_executable_file(&self.options.ffmpeg, ExecutableRole::Ffmpeg)
    }

    fn write_package(&self, staging: &Path) -> Result<(), PackageError> {
        let archive_path = staging.join(ARCHIVE_NAME);
        BrowserArchive::collect(&self.options.browser_root)?.write(&archive_path)?;
        let browser = Artifact::inspect(ARCHIVE_NAME, &archive_path)?;
        let bootstrap = Artifact::inspect("bootstrap", &self.options.bootstrap)?;
        let ffmpeg = Artifact::inspect(FFMPEG_NAME, &self.options.ffmpeg)?;
        validate_unzipped_size([&bootstrap, &browser, &ffmpeg])?;

        let package_path = staging.join(PACKAGE_NAME);
        write_zip(
            &package_path,
            &self.options.bootstrap,
            &archive_path,
            &self.options.ffmpeg,
        )?;
        let package = Artifact::inspect(PACKAGE_NAME, &package_path)?;

        PackageManifest::new(bootstrap, browser, ffmpeg, package)
            .write(&staging.join(MANIFEST_NAME))?;
        fs::remove_file(&archive_path).map_err(|source| {
            PackageError::io("remove staged browser archive", &archive_path, source)
        })
    }
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_zip(
    path: &Path,
    bootstrap: &Path,
    browser_archive: &Path,
    ffmpeg: &Path,
) -> Result<(), PackageError> {
    let file =
        File::create(path).map_err(|source| PackageError::io("create Lambda ZIP", path, source))?;
    let mut zip = ZipWriter::new(file);
    append_zip_file(
        &mut zip,
        "bootstrap",
        bootstrap,
        CompressionMethod::Deflated,
        0o755,
    )?;
    append_zip_file(
        &mut zip,
        ARCHIVE_NAME,
        browser_archive,
        CompressionMethod::Stored,
        0o644,
    )?;
    append_zip_file(
        &mut zip,
        FFMPEG_NAME,
        ffmpeg,
        CompressionMethod::Deflated,
        0o755,
    )?;
    zip.finish().map_err(PackageError::Zip)?;
    Ok(())
}

fn append_zip_file(
    zip: &mut ZipWriter<File>,
    name: &str,
    source: &Path,
    compression: CompressionMethod,
    mode: u32,
) -> Result<(), PackageError> {
    let options = SimpleFileOptions::default()
        .compression_method(compression)
        .compression_level((compression == CompressionMethod::Deflated).then_some(9))
        .last_modified_time(DateTime::default())
        .unix_permissions(mode);
    zip.start_file(name, options).map_err(PackageError::Zip)?;
    let mut input = File::open(source)
        .map_err(|error| PackageError::io("open Lambda ZIP input", source, error))?;
    io::copy(&mut input, zip)
        .map_err(|error| PackageError::io("write Lambda ZIP input", source, error))?;
    Ok(())
}

#[derive(Clone, Copy)]
enum ExecutableRole {
    Bootstrap,
    Browser,
    Ffmpeg,
}

impl ExecutableRole {
    const fn label(self) -> &'static str {
        match self {
            Self::Bootstrap => "Lambda bootstrap",
            Self::Browser => "browser executable",
            Self::Ffmpeg => "FFmpeg executable",
        }
    }

    const fn invalid_elf_reason(self) -> &'static str {
        match self {
            Self::Bootstrap => "Lambda bootstrap is not a Linux arm64 ELF executable",
            Self::Browser => "browser executable is not a Linux arm64 ELF executable",
            Self::Ffmpeg => "FFmpeg executable is not a Linux arm64 ELF executable",
        }
    }
}

fn require_executable_file(path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| PackageError::io("inspect package input", path, source))?;
    if !metadata.is_file() || !has_executable_bit(&metadata) {
        return Err(PackageError::invalid_input(path, role.label()));
    }
    if metadata.len() < ELF_HEADER_BYTES as u64 {
        return Err(PackageError::invalid_input(path, role.invalid_elf_reason()));
    }
    require_linux_arm64_elf(path, role)
}

fn require_directory(path: &Path, reason: &'static str) -> Result<(), PackageError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| PackageError::io("inspect package input", path, source))?;
    if metadata.is_dir() {
        return Ok(());
    }
    Err(PackageError::invalid_input(path, reason))
}

fn require_linux_arm64_elf(path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    let mut file = File::open(path)
        .map_err(|source| PackageError::io("open package executable", path, source))?;
    let mut header = [0_u8; ELF_HEADER_BYTES];
    file.read_exact(&mut header)
        .map_err(|source| PackageError::io("read package executable header", path, source))?;

    let machine = u16::from_le_bytes([header[18], header[19]]);
    let is_linux_arm64 = header[..4] == [0x7f, b'E', b'L', b'F']
        && header[4] == 2
        && header[5] == 1
        && machine == ELF_MACHINE_AARCH64;
    if !is_linux_arm64 {
        return Err(PackageError::invalid_input(path, role.invalid_elf_reason()));
    }
    Ok(())
}

fn validate_unzipped_size<'a>(
    artifacts: impl IntoIterator<Item = &'a Artifact>,
) -> Result<(), PackageError> {
    let actual = artifacts.into_iter().fold(0_u64, |total, artifact| {
        total.saturating_add(artifact.bytes())
    });
    if actual > MAX_UNZIPPED_PACKAGE_BYTES {
        return Err(PackageError::PackageLimit {
            actual,
            limit: MAX_UNZIPPED_PACKAGE_BYTES,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn has_executable_bit(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_executable_bit(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read as _;

    use tempfile::TempDir;
    use zip::ZipArchive;

    use super::{
        ARCHIVE_NAME, FFMPEG_NAME, MANIFEST_NAME, Options, PACKAGE_NAME, PackageBuilder,
        PackageError,
    };

    #[test]
    fn parses_options_without_imposing_argument_order() {
        let options = Options::parse(strings([
            "--output",
            "release",
            "--bootstrap",
            "bootstrap",
            "--browser-root",
            "browser",
            "--ffmpeg",
            "ffmpeg",
        ]))
        .expect("the complete options are valid");

        assert_eq!(options.bootstrap, std::path::Path::new("bootstrap"));
        assert_eq!(options.browser_root, std::path::Path::new("browser"));
        assert_eq!(options.ffmpeg, std::path::Path::new("ffmpeg"));
        assert_eq!(options.output, std::path::Path::new("release"));
    }

    #[test]
    fn produces_identical_packages_from_identical_inputs() {
        let fixture = Fixture::new();
        let first = fixture.root.path().join("first");
        let second = fixture.root.path().join("second");

        PackageBuilder::new(fixture.options(first.clone()))
            .build()
            .expect("the first package builds");
        PackageBuilder::new(fixture.options(second.clone()))
            .build()
            .expect("the second package builds");

        assert_eq!(
            read_artifact(&first, PACKAGE_NAME),
            read_artifact(&second, PACKAGE_NAME)
        );
        assert_eq!(
            read_artifact(&first, MANIFEST_NAME),
            read_artifact(&second, MANIFEST_NAME)
        );
    }

    #[test]
    fn ffmpeg_bytes_change_the_capture_environment() {
        let fixture = Fixture::new();
        let first = fixture.root.path().join("first");
        let second = fixture.root.path().join("second");
        PackageBuilder::new(fixture.options(first.clone()))
            .build()
            .expect("the first package builds");
        fs::write(&fixture.ffmpeg, arm64_elf(b"different ffmpeg"))
            .expect("the FFmpeg fixture can change");
        make_executable(&fixture.ffmpeg);
        PackageBuilder::new(fixture.options(second.clone()))
            .build()
            .expect("the second package builds");

        assert_ne!(capture_environment(&first), capture_environment(&second),);
    }

    #[test]
    fn emits_the_runtime_inputs_and_their_canonical_identities() {
        let fixture = Fixture::new();
        let output = fixture.root.path().join("release");
        PackageBuilder::new(fixture.options(output.clone()))
            .build()
            .expect("the package builds");

        let manifest: serde_json::Value =
            serde_json::from_slice(&read_artifact(&output, MANIFEST_NAME))
                .expect("the manifest is JSON");
        assert_eq!(manifest["version"], 2);
        assert_eq!(manifest["target"], "provided.al2023.arm64");
        assert!(
            manifest["captureEnvironment"]
                .as_str()
                .is_some_and(|digest| digest.starts_with("sha256:"))
        );
        assert_eq!(manifest["browserArchive"]["path"], ARCHIVE_NAME);
        assert_eq!(manifest["ffmpeg"]["path"], FFMPEG_NAME);

        let file = fs::File::open(output.join(PACKAGE_NAME)).expect("the ZIP is readable");
        let mut zip = ZipArchive::new(file).expect("the ZIP is valid");
        assert_eq!(zip.len(), 3);
        let mut browser = Vec::new();
        zip.by_name(ARCHIVE_NAME)
            .expect("the ZIP carries the browser archive")
            .read_to_end(&mut browser)
            .expect("the browser archive is readable");
        assert!(!browser.is_empty());
        assert!(zip.by_name(FFMPEG_NAME).is_ok());
    }

    #[test]
    fn refuses_to_replace_an_existing_output() {
        let fixture = Fixture::new();
        let output = fixture.root.path().join("release");
        fs::create_dir(&output).expect("the existing output is writable");

        let error = PackageBuilder::new(fixture.options(output))
            .build()
            .expect_err("an existing output must not be replaced");

        assert!(matches!(error, PackageError::OutputExists(_)));
    }

    #[test]
    fn rejects_an_executable_for_the_wrong_platform() {
        let fixture = Fixture::new();
        fs::write(&fixture.bootstrap, [0_u8; super::ELF_HEADER_BYTES])
            .expect("the fixture bootstrap is writable");

        let error = PackageBuilder::new(fixture.options(fixture.root.path().join("release")))
            .build()
            .expect_err("a non-arm64 bootstrap must be rejected");

        assert!(matches!(error, PackageError::InvalidInput { .. }));
    }

    #[test]
    fn rejects_a_truncated_executable_as_invalid_input() {
        let fixture = Fixture::new();
        fs::write(&fixture.bootstrap, b"short").expect("the fixture bootstrap is writable");

        let error = PackageBuilder::new(fixture.options(fixture.root.path().join("release")))
            .build()
            .expect_err("a truncated bootstrap must be rejected");

        assert!(matches!(error, PackageError::InvalidInput { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_links_in_the_browser_payload() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        symlink(
            fixture.browser.join("fonts/OpenSans.ttf"),
            fixture.browser.join("fonts/alias.ttf"),
        )
        .expect("the fixture link is writable");

        let error = PackageBuilder::new(fixture.options(fixture.root.path().join("release")))
            .build()
            .expect_err("a browser link must be rejected");

        assert!(matches!(error, PackageError::InvalidInput { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_linked_browser_root() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let linked_browser = fixture.root.path().join("linked-browser");
        symlink(&fixture.browser, &linked_browser).expect("the fixture link is writable");
        let mut options = fixture.options(fixture.root.path().join("release"));
        options.browser_root = linked_browser;

        let error = PackageBuilder::new(options)
            .build()
            .expect_err("a linked browser root must be rejected");

        assert!(matches!(error, PackageError::InvalidInput { .. }));
    }

    fn strings<const N: usize>(values: [&str; N]) -> impl Iterator<Item = String> {
        values.into_iter().map(String::from)
    }

    fn read_artifact(directory: &std::path::Path, name: &str) -> Vec<u8> {
        fs::read(directory.join(name)).expect("the package artifact is readable")
    }

    fn capture_environment(directory: &std::path::Path) -> String {
        let manifest: serde_json::Value =
            serde_json::from_slice(&read_artifact(directory, MANIFEST_NAME))
                .expect("the package manifest is JSON");
        manifest["captureEnvironment"]
            .as_str()
            .expect("the capture environment is a string")
            .to_owned()
    }

    struct Fixture {
        root: TempDir,
        bootstrap: std::path::PathBuf,
        browser: std::path::PathBuf,
        ffmpeg: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = TempDir::new().expect("the fixture root is writable");
            let bootstrap = root.path().join("bootstrap");
            let browser = root.path().join("browser");
            let ffmpeg = root.path().join("ffmpeg");
            fs::create_dir(&browser).expect("the browser root is writable");
            fs::create_dir(browser.join("fonts")).expect("the fonts root is writable");
            fs::write(&bootstrap, arm64_elf(b"bootstrap")).expect("the bootstrap is writable");
            fs::write(&ffmpeg, arm64_elf(b"ffmpeg")).expect("the FFmpeg fixture is writable");
            fs::write(browser.join("chrome-headless-shell"), arm64_elf(b"browser"))
                .expect("the browser executable is writable");
            fs::write(browser.join("fonts/OpenSans.ttf"), b"font").expect("the font is writable");
            make_executable(&bootstrap);
            make_executable(&ffmpeg);
            make_executable(&browser.join("chrome-headless-shell"));
            Self {
                root,
                bootstrap,
                browser,
                ffmpeg,
            }
        }

        fn options(&self, output: std::path::PathBuf) -> Options {
            Options {
                bootstrap: self.bootstrap.clone(),
                browser_root: self.browser.clone(),
                ffmpeg: self.ffmpeg.clone(),
                output,
            }
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt as _;

        let mut permissions = fs::metadata(path)
            .expect("the fixture input exists")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("the fixture mode is writable");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &std::path::Path) {}

    fn arm64_elf(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; super::ELF_HEADER_BYTES];
        bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        bytes[18..20].copy_from_slice(&super::ELF_MACHINE_AARCH64.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
}
