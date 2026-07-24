//! Owned, span-preserving syntax tree without Onmark semantic assumptions.
//!
//! Multiple roots, arbitrary element names, and text are representable so the
//! compiler—not the tokenizer—owns language legality.

use std::fmt;

use crate::model::SourceSpan;

/// A parsed screenplay fragment before language binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDocument {
    nodes: Vec<Node>,
    span: SourceSpan,
}

impl SourceDocument {
    pub(super) const fn new(nodes: Vec<Node>, span: SourceSpan) -> Self {
        Self { nodes, span }
    }

    /// Returns authored top-level nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Returns the span of the complete authored source.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(crate) fn into_parts(self) -> (Vec<Node>, SourceSpan) {
        (self.nodes, self.span)
    }
}

/// One node in the syntax tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Node {
    /// A markup element.
    Element(Element),
    /// Authored character data.
    Text(TextNode),
}

/// One element with authored attributes and children.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Element {
    name: ElementName,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
    span: SourceSpan,
}

impl Element {
    pub(super) const fn new(
        name: ElementName,
        attributes: Vec<Attribute>,
        children: Vec<Node>,
        span: SourceSpan,
    ) -> Self {
        Self {
            name,
            attributes,
            children,
            span,
        }
    }

    /// Returns the normalized authored element name.
    #[must_use]
    pub const fn name(&self) -> &ElementName {
        &self.name
    }

    /// Returns attributes in authored order.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// Returns child nodes in authored order.
    #[must_use]
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// Returns the authored element extent.
    ///
    /// A recovered unclosed element ends at the end of the source document.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(crate) fn into_parts(self) -> (ElementName, Vec<Attribute>, Vec<Node>, SourceSpan) {
        (self.name, self.attributes, self.children, self.span)
    }
}

/// HTML-normalized name of an element.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ElementName {
    local: Box<str>,
    span: SourceSpan,
}

impl ElementName {
    pub(super) const fn new(local: Box<str>, span: SourceSpan) -> Self {
        Self { local, span }
    }

    /// Returns the ASCII-lowercase HTML name.
    #[must_use]
    pub fn local(&self) -> &str {
        &self.local
    }

    /// Returns the exact authored-name span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn same_name(&self, other: &Self) -> bool {
        self.local == other.local
    }

    pub(super) fn display_name(&self) -> Box<str> {
        self.local.clone()
    }
}

impl fmt::Display for ElementName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.local)
    }
}

/// One authored attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribute {
    name: AttributeName,
    value: Box<str>,
    span: SourceSpan,
    value_span: SourceSpan,
}

impl Attribute {
    pub(super) const fn new(
        name: AttributeName,
        value: Box<str>,
        span: SourceSpan,
        value_span: SourceSpan,
    ) -> Self {
        Self {
            name,
            value,
            span,
            value_span,
        }
    }

    /// Returns the normalized authored attribute name.
    #[must_use]
    pub const fn name(&self) -> &AttributeName {
        &self.name
    }

    /// Returns the decoded attribute value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the span covering the name, equals sign, quotes, and value.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    /// Returns the span of authored bytes inside the value quotes.
    #[must_use]
    pub const fn value_span(&self) -> SourceSpan {
        self.value_span
    }
}

/// HTML-normalized name of an attribute.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AttributeName {
    local: Box<str>,
    span: SourceSpan,
}

impl AttributeName {
    pub(super) const fn new(local: Box<str>, span: SourceSpan) -> Self {
        Self { local, span }
    }

    /// Returns the ASCII-lowercase HTML name.
    #[must_use]
    pub fn local(&self) -> &str {
        &self.local
    }

    /// Returns the exact authored-name span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn same_name(&self, other: &Self) -> bool {
        self.local == other.local
    }

    pub(super) fn display_name(&self) -> Box<str> {
        self.local.clone()
    }
}

impl fmt::Display for AttributeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.local)
    }
}

/// One decoded text run and its authored source span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextNode {
    text: Box<str>,
    span: SourceSpan,
}

impl TextNode {
    pub(super) const fn new(text: Box<str>, span: SourceSpan) -> Self {
        Self { text, span }
    }

    /// Returns decoded character data.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the authored text span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(crate) fn into_parts(self) -> (Box<str>, SourceSpan) {
        (self.text, self.span)
    }
}
