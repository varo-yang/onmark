use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write};
use std::path::Path;

use onmark_core::protocol::{BundleFile, BundleManifest};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tempfile::{Builder as TempDirBuilder, TempDir};

use super::{UnitRootError, UnitRootErrorKind, UnitRootLimits};
use crate::unit_root::AssetSource;

const COPY_BUFFER_BYTES: usize = 64 * 1024;

pub(super) fn materialize(
    source_root: &Path,
    manifest: &BundleManifest,
    assets: impl Iterator<Item = AssetSource>,
    limits: UnitRootLimits,
) -> Result<TempDir, UnitRootError> {
    let assets = collect_assets(source_root, manifest, assets, limits.max_files())?;
    verify_bundle_identity(source_root, manifest)?;
    let manifest_bytes = encoded_manifest_bytes(manifest);
    let mut writer = UnitWriter::create(source_root, limits.max_bytes(), manifest_bytes)?;

    writer.copy_bundle(source_root, manifest)?;
    writer.copy_assets(&assets)?;
    writer.write_manifest(manifest)?;
    Ok(writer.finish())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleIdentity<'a> {
    // Declaration order mirrors the V1 compact-JSON identity contract.
    version: u16,
    entry_point: &'a str,
    files: &'a [BundleFile],
}

fn verify_bundle_identity(root: &Path, manifest: &BundleManifest) -> Result<(), UnitRootError> {
    let identity = BundleIdentity {
        version: manifest.version().get(),
        entry_point: manifest.entry_point(),
        files: manifest.files(),
    };
    if json_sha256(&identity) != manifest.bundle_id() {
        return Err(failure(
            UnitRootErrorKind::BundleIdentity,
            &root.join(BundleManifest::FILE_NAME),
            "bundle ID does not match its canonical payload description",
        ));
    }
    Ok(())
}

fn collect_assets(
    root: &Path,
    manifest: &BundleManifest,
    assets: impl Iterator<Item = AssetSource>,
    max_files: usize,
) -> Result<Vec<AssetSource>, UnitRootError> {
    let base_files = manifest.files().len().checked_add(1).ok_or_else(|| {
        failure(
            UnitRootErrorKind::FileLimit,
            root,
            "unit-root file count exceeds its accounting domain",
        )
    })?;
    if base_files > max_files {
        return Err(file_limit(root));
    }
    let available_assets = max_files - base_files;

    let mut seen = BTreeSet::new();
    let mut collected = Vec::new();
    for asset in assets {
        if collected.len() == available_assets {
            return Err(file_limit(root));
        }
        if !seen.insert(asset.id()) {
            return Err(failure(
                UnitRootErrorKind::DuplicateAsset,
                asset.local_path(),
                "materialized asset identity is duplicated",
            ));
        }
        collected.push(asset);
    }
    Ok(collected)
}

struct UnitWriter {
    directory: TempDir,
    budget: ByteBudget,
}

impl UnitWriter {
    fn create(
        source_root: &Path,
        max_bytes: u64,
        manifest_bytes: u64,
    ) -> Result<Self, UnitRootError> {
        let mut budget = ByteBudget::new(max_bytes);
        budget.consume(&source_root.join(BundleManifest::FILE_NAME), manifest_bytes)?;
        let directory = TempDirBuilder::new()
            .prefix("onmark-unit-")
            .tempdir()
            .map_err(|source| {
                UnitRootError::io(source_root, "failed to create a private unit root", source)
            })?;
        Ok(Self { directory, budget })
    }

    fn copy_bundle(
        &mut self,
        source_root: &Path,
        manifest: &BundleManifest,
    ) -> Result<(), UnitRootError> {
        for file in manifest.files() {
            self.copy(
                &source_root.join(file.path()),
                file.path(),
                Some(file.bytes()),
                file.sha256(),
            )?;
        }
        Ok(())
    }

    fn copy_assets(&mut self, assets: &[AssetSource]) -> Result<(), UnitRootError> {
        for asset in assets {
            let digest = asset.id().to_string();
            self.copy(
                asset.local_path(),
                asset.unit_relative_path(),
                None,
                &digest,
            )?;
        }
        Ok(())
    }

    fn copy(
        &mut self,
        source: &Path,
        destination: impl AsRef<Path>,
        expected_bytes: Option<u64>,
        expected_digest: &str,
    ) -> Result<(), UnitRootError> {
        let destination = self.directory.path().join(destination);
        let actual = copy_file(source, &destination, &self.budget)?;
        if expected_bytes.is_some_and(|expected| expected != actual.bytes) {
            return Err(failure(
                UnitRootErrorKind::SizeMismatch,
                source,
                "materialization source size differs from its manifest",
            ));
        }
        if actual.digest != expected_digest {
            return Err(failure(
                UnitRootErrorKind::DigestMismatch,
                source,
                "materialization source differs from its frozen identity",
            ));
        }
        self.budget.consume(source, actual.bytes)
    }

    fn write_manifest(&self, manifest: &BundleManifest) -> Result<(), UnitRootError> {
        let path = self.directory.path().join(BundleManifest::FILE_NAME);
        let mut output = create_destination(&path)?;
        serde_json::to_writer_pretty(&mut output, manifest).map_err(|source| {
            UnitRootError::io(
                &path,
                "failed to write unit-root manifest",
                io::Error::other(source),
            )
        })?;
        output.write_all(b"\n").map_err(|source| {
            UnitRootError::io(&path, "failed to write unit-root manifest", source)
        })
    }

    fn finish(self) -> TempDir {
        self.directory
    }
}

struct CopiedFile {
    bytes: u64,
    digest: String,
}

fn copy_file(
    source: &Path,
    destination: &Path,
    budget: &ByteBudget,
) -> Result<CopiedFile, UnitRootError> {
    reject_invalid_source(source)?;
    let mut input = File::open(source).map_err(|source_error| {
        UnitRootError::io(
            source,
            "failed to open materialization source",
            source_error,
        )
    })?;
    let mut output = create_destination(destination)?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();

    loop {
        let count = input.read(&mut buffer).map_err(|source_error| {
            UnitRootError::io(
                source,
                "failed to read materialization source",
                source_error,
            )
        })?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(count).expect("the fixed copy buffer fits in u64"))
            .ok_or_else(|| byte_limit(source))?;
        budget.ensure(source, bytes)?;
        output.write_all(&buffer[..count]).map_err(|source_error| {
            UnitRootError::io(
                destination,
                "failed to write unit-root payload",
                source_error,
            )
        })?;
        digest.update(&buffer[..count]);
    }

    Ok(CopiedFile {
        bytes,
        digest: encode_sha256(&digest.finalize()),
    })
}

fn reject_invalid_source(source: &Path) -> Result<(), UnitRootError> {
    let metadata = fs::symlink_metadata(source).map_err(|source_error| {
        UnitRootError::io(
            source,
            "failed to inspect materialization source",
            source_error,
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(failure(
            UnitRootErrorKind::InvalidSource,
            source,
            "materialization source must be a regular file, not a symlink",
        ));
    }
    Ok(())
}

fn create_destination(path: &Path) -> Result<File, UnitRootError> {
    let parent = path
        .parent()
        .expect("a unit-root destination always has a parent");
    fs::create_dir_all(parent).map_err(|source| {
        UnitRootError::io(parent, "failed to create unit-root directory", source)
    })?;
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| UnitRootError::io(path, "failed to create unit-root payload", source))
}

fn encoded_manifest_bytes(manifest: &BundleManifest) -> u64 {
    let mut counter = ByteCounter::default();
    serde_json::to_writer_pretty(&mut counter, manifest)
        .expect("a validated bundle manifest always serializes to a byte counter");
    counter
        .write_all(b"\n")
        .expect("a byte counter cannot reject manifest bytes");
    counter.finish()
}

fn json_sha256(value: &impl Serialize) -> String {
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value)
        .expect("a validated bundle identity always serializes to a digest writer");
    writer.finish()
}

fn encode_sha256(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for &byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

struct ByteBudget {
    remaining: u64,
}

impl ByteBudget {
    const fn new(limit: u64) -> Self {
        Self { remaining: limit }
    }

    fn ensure(&self, path: &Path, bytes: u64) -> Result<(), UnitRootError> {
        if bytes > self.remaining {
            return Err(byte_limit(path));
        }
        Ok(())
    }

    fn consume(&mut self, path: &Path, bytes: u64) -> Result<(), UnitRootError> {
        self.ensure(path, bytes)?;
        self.remaining -= bytes;
        Ok(())
    }
}

#[derive(Default)]
struct ByteCounter {
    bytes: u64,
}

impl ByteCounter {
    const fn finish(self) -> u64 {
        self.bytes
    }
}

impl Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let bytes = u64::try_from(buffer.len()).expect("an in-memory slice length fits in u64");
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .expect("the bounded manifest cannot overflow u64");
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct DigestWriter(Sha256);

impl DigestWriter {
    fn finish(self) -> String {
        encode_sha256(&self.0.finalize())
    }
}

impl Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn failure(kind: UnitRootErrorKind, path: &Path, message: &'static str) -> UnitRootError {
    UnitRootError::without_source(kind, path, message)
}

fn file_limit(path: &Path) -> UnitRootError {
    failure(
        UnitRootErrorKind::FileLimit,
        path,
        "unit root exceeds its configured file limit",
    )
}

fn byte_limit(path: &Path) -> UnitRootError {
    failure(
        UnitRootErrorKind::ByteLimit,
        path,
        "unit root exceeds its configured byte limit",
    )
}
