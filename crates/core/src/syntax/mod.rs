//! Span-preserving screenplay markup syntax.
//!
//! This module owns XML-compatible fragment tokenization and tree structure.
//! It does not decide which Onmark elements or relationships are meaningful.

mod builder;
mod error;
mod parser;
mod reference;
mod tree;

pub(crate) use error::{SyntaxError, SyntaxErrorKind, UnsupportedDirective};
pub(crate) use parser::parse;
pub use tree::{Attribute, AttributeName, Element, ElementName, Node, SourceDocument, TextNode};
