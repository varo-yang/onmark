use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use onmark_core::compiler::{
    Authored, ResolvedFilm, ResolvedScene, ResolvedShot, ResolvedShotContent,
};
use onmark_core::model::{AssetRef, FrozenAsset, FrozenAssetId};
use onmark_media::{Ffprobe, ProbeError};
use onmark_render::{InvalidMaterializedAsset, MaterializedAsset};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tokio::task::JoinError;

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_ASSET_FILES: usize = 10_000;
const MAX_FROZEN_BYTES: u64 = 128 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct FrozenCatalog {
    directory: TempDir,
    facts: BTreeMap<AssetRef, FrozenAsset>,
    paths: BTreeMap<AssetRef, PathBuf>,
}

impl FrozenCatalog {
    pub(super) async fn freeze(
        film: &ResolvedFilm,
        source_directory: &Path,
        ffprobe: &Ffprobe,
    ) -> Result<Self, AssetError> {
        let references = asset_references(film);
        let source_directory = source_directory.to_owned();
        let ffprobe = ffprobe.clone();
        tokio::task::spawn_blocking(move || {
            Self::freeze_references(references, &source_directory, &ffprobe)
        })
        .await
        .map_err(AssetError::Task)?
    }

    fn freeze_references(
        references: BTreeSet<AssetRef>,
        source_directory: &Path,
        ffprobe: &Ffprobe,
    ) -> Result<Self, AssetError> {
        if references.len() > MAX_ASSET_FILES {
            return Err(AssetError::TooManyFiles);
        }

        let directory = tempfile::Builder::new()
            .prefix("onmark-inputs-")
            .tempdir()
            .map_err(AssetError::TemporaryDirectory)?;
        let mut remaining_bytes = MAX_FROZEN_BYTES;
        let mut facts = BTreeMap::new();
        let mut paths = BTreeMap::new();

        for (index, reference) in references.into_iter().enumerate() {
            let source = source_directory.join(reference.as_str());
            let target = directory.path().join(format!("{index:08}"));
            let frozen_id = freeze_file(&source, &target, &mut remaining_bytes)?;
            let metadata = ffprobe.probe(&target).map_err(|error| AssetError::Probe {
                path: source,
                source: error,
            })?;

            facts.insert(reference.clone(), FrozenAsset::new(frozen_id, metadata));
            paths.insert(reference, target);
        }

        Ok(Self {
            directory,
            facts,
            paths,
        })
    }

    pub(super) const fn facts(&self) -> &BTreeMap<AssetRef, FrozenAsset> {
        &self.facts
    }

    pub(super) fn into_materialized(self) -> Result<MaterializedInputs, AssetError> {
        let Self {
            directory,
            facts,
            mut paths,
        } = self;
        let mut assets = BTreeMap::new();
        for (reference, frozen) in facts {
            let path = paths
                .remove(&reference)
                .expect("every frozen asset retains its private path");
            let asset =
                MaterializedAsset::new(frozen, path).map_err(AssetError::MaterializedAsset)?;
            assets.entry(asset.id()).or_insert(asset);
        }

        Ok(MaterializedInputs {
            assets: assets.into_values().collect(),
            directory,
        })
    }
}

#[derive(Debug)]
pub(super) struct MaterializedInputs {
    assets: Vec<MaterializedAsset>,
    directory: TempDir,
}

impl MaterializedInputs {
    pub(super) fn into_parts(self) -> (Vec<MaterializedAsset>, TempDir) {
        (self.assets, self.directory)
    }
}

#[derive(Debug)]
pub(super) enum AssetError {
    TooManyFiles,
    ByteLimit(PathBuf),
    TemporaryDirectory(io::Error),
    Task(JoinError),
    Open { path: PathBuf, source: io::Error },
    Create { path: PathBuf, source: io::Error },
    Read { path: PathBuf, source: io::Error },
    Write { path: PathBuf, source: io::Error },
    Probe { path: PathBuf, source: ProbeError },
    MaterializedAsset(InvalidMaterializedAsset),
}

impl fmt::Display for AssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyFiles => formatter.write_str("screenplay exceeds the frozen-file limit"),
            Self::ByteLimit(path) => write!(
                formatter,
                "{} exceeds the total frozen-input byte limit",
                path.display()
            ),
            Self::TemporaryDirectory(_) => {
                formatter.write_str("failed to create a private frozen-input directory")
            }
            Self::Task(_) => formatter.write_str("frozen-input work did not finish"),
            Self::Open { path, .. } => {
                write!(formatter, "failed to open asset {}", path.display())
            }
            Self::Create { path, .. } => {
                write!(
                    formatter,
                    "failed to create frozen asset {}",
                    path.display()
                )
            }
            Self::Read { path, .. } => {
                write!(formatter, "failed to read asset {}", path.display())
            }
            Self::Write { path, .. } => {
                write!(formatter, "failed to write frozen asset {}", path.display())
            }
            Self::Probe { path, .. } => {
                write!(formatter, "failed to probe asset {}", path.display())
            }
            Self::MaterializedAsset(source) => source.fmt(formatter),
        }
    }
}

impl Error for AssetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TemporaryDirectory(source)
            | Self::Open { source, .. }
            | Self::Create { source, .. }
            | Self::Read { source, .. }
            | Self::Write { source, .. } => Some(source),
            Self::Task(source) => Some(source),
            Self::Probe { source, .. } => Some(source),
            Self::MaterializedAsset(source) => Some(source),
            Self::TooManyFiles | Self::ByteLimit(_) => None,
        }
    }
}

fn asset_references(film: &ResolvedFilm) -> BTreeSet<AssetRef> {
    film.scenes()
        .iter()
        .flat_map(ResolvedScene::shots)
        .flat_map(ResolvedShot::content)
        .filter_map(content_asset)
        .cloned()
        .collect()
}

fn content_asset(content: &ResolvedShotContent) -> Option<&AssetRef> {
    match content {
        ResolvedShotContent::Video(video) => video.src().map(Authored::value),
        ResolvedShotContent::VoiceOver(voice_over) => voice_over.src().map(Authored::value),
        ResolvedShotContent::Overlay(_) => None,
    }
}

fn freeze_file(
    source: &Path,
    target: &Path,
    remaining_bytes: &mut u64,
) -> Result<FrozenAssetId, AssetError> {
    let mut input = File::open(source).map_err(|error| AssetError::Open {
        path: source.to_owned(),
        source: error,
    })?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .map_err(|error| AssetError::Create {
            path: target.to_owned(),
            source: error,
        })?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; COPY_BUFFER_BYTES].into_boxed_slice();

    loop {
        let bytes = input.read(&mut buffer).map_err(|error| AssetError::Read {
            path: source.to_owned(),
            source: error,
        })?;
        if bytes == 0 {
            break;
        }
        let bytes_u64 = u64::try_from(bytes).expect("a buffer length fits in u64");
        *remaining_bytes = remaining_bytes
            .checked_sub(bytes_u64)
            .ok_or_else(|| AssetError::ByteLimit(source.to_owned()))?;
        output
            .write_all(&buffer[..bytes])
            .map_err(|error| AssetError::Write {
                path: target.to_owned(),
                source: error,
            })?;
        digest.update(&buffer[..bytes]);
    }

    Ok(FrozenAssetId::from_sha256(digest.finalize().into()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{MAX_FROZEN_BYTES, freeze_file};

    #[test]
    fn freezes_the_exact_bytes_it_hashes() {
        let directory = tempdir().expect("the fixture directory is available");
        let source = directory.path().join("source.mp4");
        let target = directory.path().join("frozen");
        fs::write(&source, b"video bytes").expect("the fixture is writable");
        let mut remaining = MAX_FROZEN_BYTES;

        let id = freeze_file(&source, &target, &mut remaining).expect("the asset freezes");

        assert_eq!(
            fs::read(target).expect("the frozen copy is readable"),
            b"video bytes"
        );
        assert_eq!(
            id.to_string(),
            "sha256:96b050b919f3fca2fc8b6923537136a197ad13c583beb1438d1a12ccbc999c42",
        );
    }
}
