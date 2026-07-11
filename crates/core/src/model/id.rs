use std::error::Error;
use std::fmt;

/// Stable identity of an authored node within one film.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(Box<str>);

impl NodeId {
    /// Parses a node ID at the untrusted-source boundary.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidNodeId`] when the value is empty or contains ASCII
    /// whitespace, matching the HTML `id` constraint.
    pub fn parse(value: impl Into<Box<str>>) -> Result<Self, InvalidNodeId> {
        let value = value.into();

        if value.is_empty() {
            return Err(InvalidNodeId::Empty);
        }

        if value
            .chars()
            .any(|character| character.is_ascii_whitespace())
        {
            return Err(InvalidNodeId::ContainsAsciiWhitespace);
        }

        Ok(Self(value))
    }

    /// Returns the authored ID as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reason an authored node ID cannot enter the domain model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidNodeId {
    /// The ID has no characters.
    Empty,
    /// The ID contains at least one ASCII whitespace character.
    ContainsAsciiWhitespace,
}

impl fmt::Display for InvalidNodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "node ID cannot be empty",
            Self::ContainsAsciiWhitespace => "node ID cannot contain ASCII whitespace",
        };

        formatter.write_str(message)
    }
}

impl Error for InvalidNodeId {}

#[cfg(test)]
mod tests {
    use super::{InvalidNodeId, NodeId};

    #[test]
    fn accepts_a_visible_identifier() {
        let id = NodeId::parse("opening").expect("the fixture is a valid node ID");

        assert_eq!(id.as_str(), "opening");
        assert_eq!(id.to_string(), "opening");
    }

    #[test]
    fn rejects_an_empty_identifier() {
        assert_eq!(NodeId::parse(""), Err(InvalidNodeId::Empty));
    }

    #[test]
    fn rejects_whitespace() {
        assert_eq!(
            NodeId::parse("opening shot"),
            Err(InvalidNodeId::ContainsAsciiWhitespace),
        );
    }

    #[test]
    fn preserves_non_ascii_space_characters() {
        let id = NodeId::parse("opening\u{a0}shot")
            .expect("HTML IDs may contain non-ASCII space characters");

        assert_eq!(id.as_str(), "opening\u{a0}shot");
    }
}
