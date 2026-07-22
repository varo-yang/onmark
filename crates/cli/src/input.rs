//! Bounded UTF-8 file ingestion for CLI-owned text boundaries.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;
use std::string::FromUtf8Error;

pub(super) fn read_utf8(path: &Path, max_bytes: u64) -> Result<String, BoundedReadError> {
    let file = File::open(path).map_err(BoundedReadError::Open)?;
    let sentinel_limit = max_bytes
        .checked_add(1)
        .expect("CLI text limits leave room for one sentinel byte");
    let mut bytes = Vec::new();
    file.take(sentinel_limit)
        .read_to_end(&mut bytes)
        .map_err(BoundedReadError::Read)?;
    if u64::try_from(bytes.len()).expect("a buffer length fits in u64") > max_bytes {
        return Err(BoundedReadError::Limit { max_bytes });
    }
    String::from_utf8(bytes).map_err(BoundedReadError::Utf8)
}

#[derive(Debug)]
pub(super) enum BoundedReadError {
    Open(io::Error),
    Read(io::Error),
    Limit { max_bytes: u64 },
    Utf8(FromUtf8Error),
}

impl fmt::Display for BoundedReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(_) => formatter.write_str("failed to open input"),
            Self::Read(_) => formatter.write_str("failed to read input"),
            Self::Limit { max_bytes } => {
                write!(formatter, "input exceeds the {max_bytes}-byte limit")
            }
            Self::Utf8(_) => formatter.write_str("input is not valid UTF-8"),
        }
    }
}

impl Error for BoundedReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open(source) | Self::Read(source) => Some(source),
            Self::Utf8(source) => Some(source),
            Self::Limit { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{BoundedReadError, read_utf8};

    #[test]
    fn accepts_the_exact_limit_and_rejects_its_sentinel_byte() {
        let directory = tempdir().expect("the fixture directory is available");
        let exact = directory.path().join("exact.txt");
        let excessive = directory.path().join("excessive.txt");
        fs::write(&exact, "four").expect("the exact fixture is writable");
        fs::write(&excessive, "fives").expect("the excessive fixture is writable");

        assert_eq!(
            read_utf8(&exact, 4).expect("the exact limit is accepted"),
            "four",
        );
        assert!(matches!(
            read_utf8(&excessive, 4),
            Err(BoundedReadError::Limit { max_bytes: 4 })
        ));
    }
}
