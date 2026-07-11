use std::error::Error;
use std::fmt;

use super::{InvalidNodeId, NodeId};

/// Typed identity of a cue declaration or reference.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CueId(NodeId);

impl CueId {
    /// Parses a cue ID using the film-wide node-ID rules.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidNodeId`] when the value is empty or contains ASCII
    /// whitespace.
    pub fn parse(value: impl Into<Box<str>>) -> Result<Self, InvalidNodeId> {
        NodeId::parse(value).map(Self)
    }

    /// Returns the shared film-wide node identity.
    #[must_use]
    pub const fn as_node_id(&self) -> &NodeId {
        &self.0
    }

    /// Returns the authored cue ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<NodeId> for CueId {
    fn from(id: NodeId) -> Self {
        Self(id)
    }
}

impl fmt::Display for CueId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Resolved source of a temporal event.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EventRef {
    /// An authored event declared by a screenplay cue.
    Cue(CueId),
}

/// Typed authored reference to a media artifact without performing IO.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssetRef(Box<str>);

impl AssetRef {
    /// Parses an opaque authored media reference.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidAssetRef::Empty`] when no artifact was authored.
    pub fn parse(value: impl Into<Box<str>>) -> Result<Self, InvalidAssetRef> {
        let value = value.into();
        if value.is_empty() {
            return Err(InvalidAssetRef::Empty);
        }
        Ok(Self(value))
    }

    /// Returns the authored media reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reason an authored asset reference is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidAssetRef {
    /// No artifact reference was authored.
    Empty,
}

impl fmt::Display for InvalidAssetRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("asset reference cannot be empty")
    }
}

impl Error for InvalidAssetRef {}

#[cfg(test)]
mod tests {
    use super::{AssetRef, CueId, InvalidAssetRef};

    #[test]
    fn parses_typed_references_once() {
        assert_eq!(
            CueId::parse("offer").expect("the cue ID is valid").as_str(),
            "offer"
        );
        assert_eq!(
            AssetRef::parse("product clip.mp4")
                .expect("asset references preserve spaces")
                .as_str(),
            "product clip.mp4",
        );
        assert_eq!(AssetRef::parse(""), Err(InvalidAssetRef::Empty));
    }
}
