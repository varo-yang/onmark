//! Stable artifact identities and the reviewed package manifest.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::error::PackageError;
use super::{CAPTURE_POLICY, PACKAGE_TARGET};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PackageManifest {
    version: u8,
    target: &'static str,
    browser_launch_policy: &'static str,
    capture_environment: String,
    bootstrap: Artifact,
    browser_archive: Artifact,
    ffmpeg: Artifact,
    lambda_zip: Artifact,
}

impl PackageManifest {
    pub(super) fn new(
        bootstrap: Artifact,
        browser: Artifact,
        ffmpeg: Artifact,
        package: Artifact,
    ) -> Self {
        let capture_environment = capture_environment(&bootstrap, &browser, &ffmpeg);
        Self {
            version: 2,
            target: PACKAGE_TARGET,
            browser_launch_policy: CAPTURE_POLICY,
            capture_environment,
            bootstrap,
            browser_archive: browser,
            ffmpeg,
            lambda_zip: package,
        }
    }

    pub(super) fn write(&self, path: &Path) -> Result<(), PackageError> {
        let mut contents = serde_json::to_vec_pretty(self).map_err(PackageError::Json)?;
        contents.push(b'\n');
        fs::write(path, contents)
            .map_err(|source| PackageError::io("write package manifest", path, source))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Artifact {
    path: &'static str,
    bytes: u64,
    sha256: String,
}

impl Artifact {
    pub(super) fn inspect(path: &'static str, source: &Path) -> Result<Self, PackageError> {
        Ok(Self {
            path,
            bytes: file_size(source, "inspect packaged artifact")?,
            sha256: digest_file(source)?,
        })
    }

    pub(super) const fn bytes(&self) -> u64 {
        self.bytes
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureEnvironment<'a> {
    version: u8,
    target: &'static str,
    browser_launch_policy: &'static str,
    bootstrap_sha256: &'a str,
    browser_archive_sha256: &'a str,
    ffmpeg_sha256: &'a str,
}

fn capture_environment(bootstrap: &Artifact, browser: &Artifact, ffmpeg: &Artifact) -> String {
    let facts = CaptureEnvironment {
        version: 2,
        target: PACKAGE_TARGET,
        browser_launch_policy: CAPTURE_POLICY,
        bootstrap_sha256: &bootstrap.sha256,
        browser_archive_sha256: &browser.sha256,
        ffmpeg_sha256: &ffmpeg.sha256,
    };
    let bytes = serde_json::to_vec(&facts).expect("capture-environment facts are infallible JSON");
    format_digest(&Sha256::digest(bytes))
}

pub(super) fn file_size(path: &Path, operation: &'static str) -> Result<u64, PackageError> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|source| PackageError::io(operation, path, source))
}

fn digest_file(path: &Path) -> Result<String, PackageError> {
    let mut file = File::open(path)
        .map_err(|source| PackageError::io("open artifact for hashing", path, source))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| PackageError::io("hash artifact", path, source))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format_digest(&digest.finalize()))
}

fn format_digest(bytes: &[u8]) -> String {
    let mut digest = String::with_capacity("sha256:".len() + bytes.len() * 2);
    digest.push_str("sha256:");
    for byte in bytes {
        write!(&mut digest, "{byte:02x}").expect("writing to a String cannot fail");
    }
    digest
}
