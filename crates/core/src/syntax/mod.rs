//! Span-preserving screenplay markup syntax.
//!
//! This module owns XML-compatible fragment tokenization and tree structure.
//! It does not decide which Onmark elements or relationships are meaningful.

mod builder;
mod error;
mod parser;
mod reference;
mod tree;

pub(crate) use error::{SyntaxError, SyntaxErrorKind, SyntaxResource, UnsupportedDirective};
pub(crate) use parser::parse;
pub use tree::{Attribute, AttributeName, Element, ElementName, Node, SourceDocument, TextNode};

/// Maximum UTF-8 bytes accepted by the screenplay syntax boundary.
///
/// Filesystem callers should enforce the same limit before allocating the
/// complete source buffer. The pure parser repeats the check for library
/// callers that already own their input text.
pub const MAX_SCREENPLAY_BYTES: usize = 8 * 1024 * 1024;
