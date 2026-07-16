//! Canonical browser traversal and tar-zstd encoding.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use onmark_aws_lambda::{
    BROWSER_ARCHIVE_EXECUTABLE, BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES, BROWSER_ARCHIVE_MAX_ENTRIES,
    BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
};
use tar::{Builder, EntryType, Header};

use super::error::PackageError;
use super::manifest::file_size;

const ZSTD_LEVEL: i32 = 19;

pub(super) struct BrowserArchive {
    entries: Vec<BrowserEntry>,
}

impl BrowserArchive {
    pub(super) fn collect(root: &Path) -> Result<Self, PackageError> {
        let mut entries = collect_files(root)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        validate_budget(&entries)?;
        Ok(Self { entries })
    }

    pub(super) fn write(&self, path: &Path) -> Result<(), PackageError> {
        let file = File::create(path)
            .map_err(|source| PackageError::io("create browser archive", path, source))?;
        let mut encoder = zstd::Encoder::new(file, ZSTD_LEVEL)
            .map_err(|source| PackageError::io("start browser compression", path, source))?;
        encoder
            .include_checksum(true)
            .map_err(|source| PackageError::io("configure browser compression", path, source))?;
        encoder
            .include_contentsize(true)
            .map_err(|source| PackageError::io("configure browser compression", path, source))?;

        let mut archive = Builder::new(encoder);
        for entry in &self.entries {
            entry.append_to(&mut archive)?;
        }
        let encoder = archive
            .into_inner()
            .map_err(|source| PackageError::io("finish browser tar", path, source))?;
        encoder
            .finish()
            .map_err(|source| PackageError::io("finish browser compression", path, source))?;

        let size = file_size(path, "inspect browser archive")?;
        if size > BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES {
            return Err(PackageError::CompressedLimit {
                actual: size,
                limit: BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES,
            });
        }
        Ok(())
    }
}

struct BrowserEntry {
    path: Box<str>,
    source: PathBuf,
    size: u64,
}

impl BrowserEntry {
    fn new(root: &Path, source: PathBuf, size: u64) -> Result<Self, PackageError> {
        let relative = source
            .strip_prefix(root)
            .expect("collected browser files remain below their root");
        let path = portable_path(relative).ok_or_else(|| {
            PackageError::invalid_input(&source, "browser archive paths must be portable UTF-8")
        })?;
        Ok(Self {
            path: path.into_boxed_str(),
            source,
            size,
        })
    }

    fn append_to<W: Write>(&self, archive: &mut Builder<W>) -> Result<(), PackageError> {
        let mut source = File::open(&self.source)
            .map_err(|error| PackageError::io("open browser archive input", &self.source, error))?;
        let mut header = Header::new_gnu();
        header.set_path(self.path.as_ref()).map_err(|error| {
            PackageError::io("encode browser archive path", &self.source, error)
        })?;
        header.set_entry_type(EntryType::Regular);
        header.set_size(self.size);
        header.set_mode(entry_mode(&self.path));
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        archive
            .append(&header, &mut source)
            .map_err(|error| PackageError::io("append browser archive entry", &self.source, error))
    }
}

fn collect_files(root: &Path) -> Result<Vec<BrowserEntry>, PackageError> {
    let mut directories = vec![root.to_owned()];
    let mut entries = Vec::new();

    while let Some(directory) = directories.pop() {
        let children = fs::read_dir(&directory)
            .map_err(|source| PackageError::io("read browser directory", &directory, source))?;
        for child in children {
            let child = child.map_err(|source| {
                PackageError::io("read browser directory entry", &directory, source)
            })?;
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|source| PackageError::io("inspect browser input", &path, source))?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() || (!file_type.is_file() && !file_type.is_dir()) {
                return Err(PackageError::invalid_input(
                    path,
                    "browser input contains a link or special file",
                ));
            }
            if file_type.is_dir() {
                directories.push(path);
                continue;
            }
            entries.push(BrowserEntry::new(root, path, metadata.len())?);
        }
    }
    Ok(entries)
}

fn portable_path(path: &Path) -> Option<String> {
    let mut portable = String::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return None;
        };
        let component = component.to_str()?;
        if !portable.is_empty() {
            portable.push('/');
        }
        portable.push_str(component);
    }
    (!portable.is_empty()).then_some(portable)
}

fn validate_budget(entries: &[BrowserEntry]) -> Result<(), PackageError> {
    if entries.len() > BROWSER_ARCHIVE_MAX_ENTRIES {
        return Err(PackageError::EntryLimit {
            actual: entries.len(),
            limit: BROWSER_ARCHIVE_MAX_ENTRIES,
        });
    }

    let expanded = entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.size)
            .ok_or(PackageError::ExpandedLimit {
                actual: u64::MAX,
                limit: BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
            })
    })?;
    if expanded > BROWSER_ARCHIVE_MAX_EXPANDED_BYTES {
        return Err(PackageError::ExpandedLimit {
            actual: expanded,
            limit: BROWSER_ARCHIVE_MAX_EXPANDED_BYTES,
        });
    }
    Ok(())
}

fn entry_mode(path: &str) -> u32 {
    if path == BROWSER_ARCHIVE_EXECUTABLE {
        0o755
    } else {
        0o644
    }
}
