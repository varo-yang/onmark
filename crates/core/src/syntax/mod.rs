//! Span-preserving authored HTML syntax.
//!
//! This module owns HTML token adaptation and strict tree structure. It does
//! not decide which Onmark elements or relationships are meaningful.

mod builder;
mod error;
mod parser;
mod tree;

pub(crate) use error::{SyntaxError, SyntaxErrorKind, SyntaxResource};
pub(crate) use parser::parse;
pub use tree::{Attribute, AttributeName, Element, ElementName, Node, SourceDocument, TextNode};

/// Maximum UTF-8 bytes accepted by the screenplay syntax boundary.
///
/// Filesystem callers should enforce the same limit before allocating the
/// complete source buffer. The pure parser repeats the check for library
/// callers that already own their input text.
pub const MAX_SCREENPLAY_BYTES: usize = 8 * 1024 * 1024;
