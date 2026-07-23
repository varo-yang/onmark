//! Deterministic assembly of generated npm platform sidecars.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::Builder as TempBuilder;

use super::artifact::{Artifact, enforce_size, parse_source_revision};
use super::error::PackageError;
use super::media::{MediaBundle, MediaFileKind};
use super::target::{ExecutableRole, ReleaseTarget};

const MAX_PACKAGE_BYTES: u64 = 384 * 1024 * 1024;
const MAX_LICENSE_BYTES: u64 = 1024 * 1024;
const MANIFEST_NAME: &str = "onmark-release.json";

pub(super) fn run(
    repository: &Path,
    arguments: impl Iterator<Item = String>,
) -> Result<(), PackageError> {
    SidecarBuilder::new(repository, Options::parse(arguments)?).build()
}

#[derive(Debug)]
struct Options {
    target: ReleaseTarget,
    onmark: PathBuf,
    media: PathBuf,
    source_revision: Box<str>,
    output: PathBuf,
}

impl Options {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, PackageError> {
        let mut values = OptionValues::default();
        let mut arguments = arguments;

        while let Some(flag) = arguments.next() {
            let Some(value) = arguments.next() else {
                return Err(PackageError::InvalidOptions(
                    format!("{flag} requires a value").into_boxed_str(),
                ));
            };
            values.set(&flag, value)?;
        }
        values.finish()
    }
}

#[derive(Default)]
struct OptionValues {
    target: Option<ReleaseTarget>,
    onmark: Option<PathBuf>,
    media: Option<PathBuf>,
    source_revision: Option<Box<str>>,
    output: Option<PathBuf>,
}

impl OptionValues {
    fn set(&mut self, flag: &str, value: String) -> Result<(), PackageError> {
        match flag {
            "--target" => set_once(&mut self.target, ReleaseTarget::parse(&value)?, flag),
            "--onmark" => set_once(&mut self.onmark, PathBuf::from(value), flag),
            "--media" => set_once(&mut self.media, PathBuf::from(value), flag),
            "--source-revision" => set_once(
                &mut self.source_revision,
                parse_source_revision(value, flag)?,
                flag,
            ),
            "--output" => set_once(&mut self.output, PathBuf::from(value), flag),
            _ => Err(PackageError::InvalidOptions(
                format!("unknown option {flag}").into_boxed_str(),
            )),
        }
    }

    fn finish(self) -> Result<Options, PackageError> {
        Ok(Options {
            target: required(self.target, "--target")?,
            onmark: required(self.onmark, "--onmark")?,
            media: required(self.media, "--media")?,
            source_revision: required(self.source_revision, "--source-revision")?,
            output: required(self.output, "--output")?,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), PackageError> {
    if slot.replace(value).is_some() {
        return Err(PackageError::InvalidOptions(
            format!("duplicate option {flag}").into_boxed_str(),
        ));
    }
    Ok(())
}

fn required<T>(value: Option<T>, flag: &str) -> Result<T, PackageError> {
    value.ok_or_else(|| {
        PackageError::InvalidOptions(format!("missing option {flag}").into_boxed_str())
    })
}

struct SidecarBuilder<'a> {
    repository: &'a Path,
    options: Options,
}

impl<'a> SidecarBuilder<'a> {
    const fn new(repository: &'a Path, options: Options) -> Self {
        Self {
            repository,
            options,
        }
    }

    fn build(self) -> Result<(), PackageError> {
        let media = self.validate_inputs()?;
        let output_parent = output_parent(&self.options.output);
        fs::create_dir_all(output_parent)
            .map_err(|source| PackageError::io("create output parent", output_parent, source))?;
        if self.options.output.exists() {
            return Err(PackageError::OutputExists(self.options.output));
        }

        let staging = TempBuilder::new()
            .prefix(".onmark-sidecar-")
            .tempdir_in(output_parent)
            .map_err(|source| {
                PackageError::io("create sidecar staging directory", output_parent, source)
            })?;
        self.write_sidecar(staging.path(), &media)?;
        fs::rename(staging.path(), &self.options.output)
            .map_err(|source| PackageError::io("publish sidecar", &self.options.output, source))?;
        let _published = staging.keep();
        Ok(())
    }

    fn validate_inputs(&self) -> Result<MediaBundle, PackageError> {
        let target = self.options.target;
        ReleaseTarget::validate_contract(self.repository)?;
        let media = MediaBundle::admit(self.repository, &self.options.media, target)?;
        let mut total = [
            target.validate_executable(&self.options.onmark, ExecutableRole::Onmark)?,
            require_regular_file(
                &self.repository.join("LICENSE"),
                MAX_LICENSE_BYTES,
                "Onmark license is not a bounded regular file",
            )?,
        ]
        .into_iter()
        .fold(0_u64, u64::saturating_add);
        for file in media.files() {
            total = total.saturating_add(require_regular_file(
                &file.source,
                MAX_PACKAGE_BYTES,
                "media payload is not a bounded regular file",
            )?);
        }

        if total > MAX_PACKAGE_BYTES {
            return Err(PackageError::PackageLimit {
                actual: total,
                limit: MAX_PACKAGE_BYTES,
            });
        }
        Ok(media)
    }

    fn write_sidecar(&self, staging: &Path, media: &MediaBundle) -> Result<(), PackageError> {
        let target = self.options.target;
        let bin = staging.join("bin");
        let licenses = staging.join("licenses");
        let sources = staging.join("sources");
        fs::create_dir_all(&bin)
            .map_err(|source| PackageError::io("create sidecar bin directory", &bin, source))?;
        fs::create_dir_all(&licenses).map_err(|source| {
            PackageError::io("create sidecar license directory", &licenses, source)
        })?;
        fs::create_dir_all(&sources).map_err(|source| {
            PackageError::io("create sidecar source directory", &sources, source)
        })?;

        let mut artifacts = Vec::new();
        let onmark = PathBuf::from("bin").join(target.executable_name(ExecutableRole::Onmark));
        copy_executable(&self.options.onmark, &staging.join(&onmark))?;
        artifacts.push(onmark);
        for file in media.files() {
            let destination = staging.join(&file.destination);
            match file.kind {
                MediaFileKind::Data => {
                    copy_file(&file.source, &destination, "copy release media file")?;
                }
                MediaFileKind::Executable => {
                    copy_executable(&file.source, &destination)?;
                }
            }
            artifacts.push(file.destination.clone());
        }

        let package = PathBuf::from("package.json");
        write_json(&staging.join(&package), &PackageJson::new(target))?;
        artifacts.push(package);

        let license = PathBuf::from("LICENSE");
        let license_path = staging.join(&license);
        fs::write(&license_path, aggregate_license(media))
            .map_err(|source| PackageError::io("write aggregate license", &license_path, source))?;
        artifacts.push(license);

        let onmark_license = PathBuf::from("licenses/Onmark.txt");
        copy_file(
            &self.repository.join("LICENSE"),
            &staging.join(&onmark_license),
            "copy release license",
        )?;
        artifacts.push(onmark_license);
        artifacts.sort();

        let manifest = PathBuf::from(MANIFEST_NAME);
        PackageManifest::collect(
            staging,
            target,
            &self.options.source_revision,
            media.ffmpeg_version(),
            media.x264_revision(),
            &artifacts,
        )?
        .write(&staging.join(&manifest))?;
        artifacts.push(manifest);
        enforce_size(staging, &artifacts, MAX_PACKAGE_BYTES)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageManifest<'a> {
    schema_version: u8,
    package_name: &'static str,
    version: &'static str,
    target: ReleaseTarget,
    source_revision: &'a str,
    ffmpeg: FfmpegProvenance<'a>,
    files: Vec<Artifact>,
}

impl<'a> PackageManifest<'a> {
    fn collect(
        root: &Path,
        target: ReleaseTarget,
        source_revision: &'a str,
        ffmpeg_version: &'a str,
        x264_revision: &'a str,
        relative_paths: &[PathBuf],
    ) -> Result<Self, PackageError> {
        let files = relative_paths
            .iter()
            .map(|path| Artifact::inspect(root, path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            schema_version: 1,
            package_name: target.package_name(),
            version: env!("CARGO_PKG_VERSION"),
            target,
            source_revision,
            ffmpeg: FfmpegProvenance {
                version: ffmpeg_version,
                x264_revision,
                build: "media-build.txt",
                source_manifest: "media-sources.json",
                sources: "sources/",
                license: "licenses/FFmpeg-GPLv2.txt",
            },
            files,
        })
    }

    fn write(&self, path: &Path) -> Result<(), PackageError> {
        write_json(path, self)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegProvenance<'a> {
    version: &'a str,
    x264_revision: &'a str,
    build: &'static str,
    source_manifest: &'static str,
    sources: &'static str,
    license: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    name: &'static str,
    version: &'static str,
    description: &'static str,
    license: &'static str,
    os: [&'static str; 1],
    cpu: [&'static str; 1],
    files: [&'static str; 7],
    exports: PackageExports,
    publish_config: PublishConfig,
}

impl PackageJson {
    const fn new(target: ReleaseTarget) -> Self {
        Self {
            name: target.package_name(),
            version: env!("CARGO_PKG_VERSION"),
            description: "Native Onmark video compiler and release media tools",
            license: "SEE LICENSE IN LICENSE",
            os: [target.operating_system()],
            cpu: [target.architecture()],
            files: [
                "bin",
                "licenses",
                "sources",
                "LICENSE",
                "media-build.txt",
                "media-sources.json",
                MANIFEST_NAME,
            ],
            exports: PackageExports {
                package: "./package.json",
                manifest: "./onmark-release.json",
            },
            publish_config: PublishConfig { access: "public" },
        }
    }
}

#[derive(Serialize)]
struct PackageExports {
    #[serde(rename = "./package.json")]
    package: &'static str,
    #[serde(rename = "./onmark-release.json")]
    manifest: &'static str,
}

#[derive(Serialize)]
struct PublishConfig {
    access: &'static str,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), PackageError> {
    let mut contents = serde_json::to_string_pretty(value)?;
    contents.push('\n');
    fs::write(path, contents)
        .map_err(|source| PackageError::io("write release metadata", path, source))
}

fn aggregate_license(media: &MediaBundle) -> String {
    format!(
        "Onmark\n\
         ======\n\
         Onmark is distributed under the terms in licenses/Onmark.txt.\n\n\
         FFmpeg, x264, and zlib\n\
         =====================\n\
         FFmpeg version: {}\n\
         x264 revision: {}\n\
         Exact source archives: sources/\n\
         Build script: sources/build-media.sh\n\
         Build record: media-build.txt\n\
         Source identities: media-sources.json\n\
         License terms: licenses/FFmpeg-GPLv2.txt, licenses/x264-GPLv2.txt, \
         and licenses/zlib.txt\n",
        media.ffmpeg_version(),
        media.x264_revision()
    )
}

fn require_regular_file(
    path: &Path,
    limit: u64,
    reason: &'static str,
) -> Result<u64, PackageError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| PackageError::io("inspect release input", path, source))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(PackageError::invalid_input(path, reason));
    }
    Ok(metadata.len())
}

fn copy_executable(source: &Path, destination: &Path) -> Result<(), PackageError> {
    copy_file(source, destination, "copy release executable")?;
    set_executable(destination)
}

fn copy_file(
    source: &Path,
    destination: &Path,
    operation: &'static str,
) -> Result<(), PackageError> {
    fs::copy(source, destination)
        .map_err(|error| PackageError::io(operation, destination, error))?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), PackageError> {
    use std::os::unix::fs::PermissionsExt as _;

    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|source| PackageError::io("set executable permissions", path, source))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), PackageError> {
    Ok(())
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{Options, PackageError, ReleaseTarget, SidecarBuilder};

    #[test]
    fn parses_options_without_imposing_argument_order() {
        let options = Options::parse(strings([
            "--output",
            "release",
            "--target",
            "darwin-arm64",
            "--onmark",
            "onmark",
            "--media",
            "media",
            "--source-revision",
            "abcdef",
        ]))
        .expect("complete release options are valid");

        assert_eq!(options.target, ReleaseTarget::DarwinArm64);
        assert_eq!(options.output, Path::new("release"));
        assert_eq!(options.media, Path::new("media"));
    }

    #[test]
    fn produces_identical_sidecars_from_identical_inputs() {
        let fixture = Fixture::new();
        let first = fixture.root.path().join("first");
        let second = fixture.root.path().join("second");

        fixture.build(first.clone());
        fixture.build(second.clone());

        assert_eq!(snapshot(&first), snapshot(&second));
    }

    #[test]
    fn writes_platform_metadata_and_hashed_provenance() {
        let fixture = Fixture::new();
        let output = fixture.root.path().join("release");
        fixture.build(output.clone());

        let package = read_json(&output.join("package.json"));
        assert_eq!(package["name"], "@onmark/cli-darwin-arm64");
        assert_eq!(package["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(package["os"], serde_json::json!(["darwin"]));
        assert_eq!(package["cpu"], serde_json::json!(["arm64"]));
        assert_eq!(package["publishConfig"]["access"], "public");

        let manifest = read_json(&output.join("onmark-release.json"));
        assert_eq!(manifest["target"], "darwin-arm64");
        assert_eq!(manifest["sourceRevision"], "abcdef");
        assert_eq!(manifest["ffmpeg"]["version"], "8.1.2");
        assert_eq!(
            manifest["ffmpeg"]["x264Revision"],
            "b35605ace3ddf7c1a5d67a2eb553f034aef41d55"
        );
        assert_eq!(manifest["ffmpeg"]["license"], "licenses/FFmpeg-GPLv2.txt");
        assert_eq!(
            manifest["files"].as_array().map(Vec::len),
            Some(15),
            "the manifest owns every payload file except itself"
        );
    }

    #[test]
    fn refuses_to_replace_an_existing_output() {
        let fixture = Fixture::new();
        let output = fixture.root.path().join("release");
        fs::create_dir(&output).expect("the conflicting output is created");

        let error = SidecarBuilder::new(fixture.root.path(), fixture.options(output.clone()))
            .build()
            .expect_err("an existing release output is rejected");

        assert!(matches!(error, PackageError::OutputExists(path) if path == output));
    }

    #[test]
    fn rejects_media_source_bytes_outside_the_admitted_identity() {
        let fixture = Fixture::new();
        let source = fixture.media.join("sources/ffmpeg.tar.xz");
        fs::write(&source, "different source").expect("the media source is changed");

        let error = SidecarBuilder::new(
            fixture.root.path(),
            fixture.options(fixture.root.path().join("release")),
        )
        .build()
        .expect_err("unadmitted media source bytes are rejected");

        assert!(matches!(
            error,
            PackageError::InvalidInput { path, reason }
                if path == source && reason == "media source does not match its admitted identity"
        ));
    }

    struct Fixture {
        root: TempDir,
        executable: PathBuf,
        media: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = TempDir::new().expect("the release fixture root is created");
            fs::write(root.path().join("LICENSE"), "Onmark license\n")
                .expect("the Onmark license is written");
            write_release_contract(root.path());
            let executable = root.path().join("executable");
            write_mach_o(&executable);
            let media = root.path().join("media");
            write_media(root.path(), &media, &executable);
            Self {
                root,
                executable,
                media,
            }
        }

        fn options(&self, output: PathBuf) -> Options {
            Options {
                target: ReleaseTarget::DarwinArm64,
                onmark: self.executable.clone(),
                media: self.media.clone(),
                source_revision: "abcdef".into(),
                output,
            }
        }

        fn build(&self, output: PathBuf) {
            SidecarBuilder::new(self.root.path(), self.options(output))
                .build()
                .expect("the sidecar builds");
        }
    }

    fn write_mach_o(path: &Path) {
        let bytes = [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1];
        fs::write(path, bytes).expect("the executable fixture is written");
        set_fixture_executable(path);
    }

    fn write_release_contract(repository: &Path) {
        let directory = repository.join("packages/launcher");
        fs::create_dir_all(&directory).expect("the launcher fixture directory is created");
        let contract = serde_json::json!({
            "schemaVersion": 1,
            "browserBuild": "149.0.7827.55",
            "targets": {
                "darwin-arm64": {},
                "linux-x64": {},
                "win32-x64": {}
            }
        });
        fs::write(
            directory.join("desktop-release.json"),
            serde_json::to_vec(&contract).expect("the release contract serializes"),
        )
        .expect("the release contract is written");
    }

    fn write_media(repository: &Path, media: &Path, executable: &Path) {
        let bin = media.join("bin");
        let licenses = media.join("licenses");
        let sources = media.join("sources");
        fs::create_dir_all(&bin).expect("the media bin directory is created");
        fs::create_dir_all(&licenses).expect("the media license directory is created");
        fs::create_dir_all(&sources).expect("the media source directory is created");
        for name in ["ffmpeg", "ffprobe"] {
            fs::copy(executable, bin.join(name)).expect("the media executable is written");
        }
        for name in ["FFmpeg-GPLv2.txt", "x264-GPLv2.txt", "zlib.txt"] {
            fs::write(licenses.join(name), format!("{name}\n"))
                .expect("the media license is written");
        }
        fs::write(media.join("build.txt"), "FFmpeg version 8.1.2\n")
            .expect("the media build record is written");
        let release = repository.join("scripts/release");
        fs::create_dir_all(&release).expect("the repository release directory is created");
        let build_script = "#!/usr/bin/env bash\nexit 0\n";
        fs::write(release.join("build-media.sh"), build_script)
            .expect("the canonical media build script is written");
        fs::write(sources.join("build-media.sh"), build_script)
            .expect("the supplied media build script is written");

        let source_specs = [
            ("ffmpeg.tar.xz", b"ffmpeg source".as_slice()),
            ("x264.tar.bz2", b"x264 source".as_slice()),
            ("zlib.tar.xz", b"zlib source".as_slice()),
        ];
        for (name, bytes) in source_specs {
            fs::write(sources.join(name), bytes).expect("the media source is written");
        }
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "ffmpeg": {
                "version": "8.1.2",
                "name": "ffmpeg.tar.xz",
                "url": "https://example.test/ffmpeg",
                "bytes": 13,
                "sha256": sha256(b"ffmpeg source")
            },
            "x264": {
                "revision": "b35605ace3ddf7c1a5d67a2eb553f034aef41d55",
                "name": "x264.tar.bz2",
                "url": "https://example.test/x264",
                "bytes": 11,
                "sha256": sha256(b"x264 source")
            },
            "zlib": {
                "version": "1.3.1",
                "name": "zlib.tar.xz",
                "url": "https://example.test/zlib",
                "bytes": 11,
                "sha256": sha256(b"zlib source")
            }
        });
        let mut contents =
            serde_json::to_string_pretty(&manifest).expect("the source manifest serializes");
        contents.push('\n');
        let canonical = release.join("media-sources.json");
        fs::write(&canonical, &contents).expect("the canonical source manifest is written");
        fs::write(media.join("media-sources.json"), contents)
            .expect("the supplied source manifest is written");
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest as _, Sha256};

        format!("{:x}", Sha256::digest(bytes))
    }

    #[cfg(unix)]
    fn set_fixture_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("the fixture is executable");
    }

    #[cfg(not(unix))]
    fn set_fixture_executable(_path: &Path) {}

    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(root, root, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(path).expect("the JSON artifact is readable"))
            .expect("the release artifact is valid JSON")
    }

    fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).expect("the sidecar directory is readable") {
            let path = entry.expect("the sidecar entry is readable").path();
            if path.is_dir() {
                collect_files(root, &path, files);
            } else {
                files.push((
                    path.strip_prefix(root)
                        .expect("the artifact is below the sidecar root")
                        .to_path_buf(),
                    fs::read(path).expect("the sidecar artifact is readable"),
                ));
            }
        }
    }

    fn strings<const N: usize>(values: [&str; N]) -> impl Iterator<Item = String> {
        values.into_iter().map(String::from)
    }
}
