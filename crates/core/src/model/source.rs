use std::error::Error;
use std::fmt;

/// Stable identity of one source document in a compilation input.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u32);

impl SourceId {
    /// Creates a source ID from its exact integer representation.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the exact integer representation.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// UTF-8 byte offset within one source document.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteOffset(u64);

impl ByteOffset {
    /// The first byte of every source document.
    pub const ZERO: Self = Self(0);

    /// Creates an offset from its exact integer representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Half-open UTF-8 byte range `[start, end)` in one source document.
///
/// This value proves only that `start <= end`. The owner of the source text
/// must verify document bounds and UTF-8 character boundaries before slicing
/// a Rust string with these offsets.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    source: SourceId,
    start: ByteOffset,
    end: ByteOffset,
}

impl SourceSpan {
    /// Creates a source span whose end is not before its start.
    ///
    /// Empty spans are valid and represent an insertion point.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSourceSpan`] when `end` is before `start`.
    pub const fn new(
        source: SourceId,
        start: ByteOffset,
        end: ByteOffset,
    ) -> Result<Self, InvalidSourceSpan> {
        if end.0 < start.0 {
            return Err(InvalidSourceSpan { source, start, end });
        }

        Ok(Self { source, start, end })
    }

    /// Returns the source document containing this span.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Returns the inclusive start byte.
    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    /// Returns the exclusive end byte.
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }

    /// Returns whether the span contains no source bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }
}

/// A source span whose end precedes its start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSourceSpan {
    source: SourceId,
    start: ByteOffset,
    end: ByteOffset,
}

impl InvalidSourceSpan {
    /// Returns the rejected source ID.
    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    /// Returns the rejected start byte.
    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    /// Returns the rejected end byte.
    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }
}

impl fmt::Display for InvalidSourceSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "source {} span ends at byte {} before it starts at byte {}",
            self.source.get(),
            self.end.get(),
            self.start.get(),
        )
    }
}

impl Error for InvalidSourceSpan {}

#[cfg(test)]
mod tests {
    use super::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};

    #[test]
    fn measures_utf8_source_in_bytes() {
        let source = "片头";
        assert_eq!(source.len(), 6);

        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::new(6))
            .expect("the fixture has ordered byte bounds");

        assert_eq!(span.start(), ByteOffset::new(0));
        assert_eq!(span.end(), ByteOffset::new(6));
        assert!(!span.is_empty());
    }

    #[test]
    fn allows_an_empty_insertion_span() {
        let span = SourceSpan::new(SourceId::new(1), ByteOffset::new(12), ByteOffset::new(12))
            .expect("equal bounds form an insertion point");

        assert!(span.is_empty());
    }

    #[test]
    fn rejects_reversed_source_bounds() {
        assert_eq!(
            SourceSpan::new(SourceId::new(2), ByteOffset::new(20), ByteOffset::new(10),),
            Err(InvalidSourceSpan {
                source: SourceId::new(2),
                start: ByteOffset::new(20),
                end: ByteOffset::new(10),
            }),
        );
    }
}
