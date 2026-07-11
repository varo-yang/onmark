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

    /// Returns the authored qualified element name.
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
}

/// Case-sensitive qualified name of an element.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ElementName {
    qualified: QualifiedName,
}

impl ElementName {
    pub(super) fn new(prefix: Option<Box<str>>, local: Box<str>, span: SourceSpan) -> Self {
        Self {
            qualified: QualifiedName::new(prefix, local, span),
        }
    }

    /// Returns the optional authored prefix without resolving namespaces.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.qualified.prefix()
    }

    /// Returns the authored local name.
    #[must_use]
    pub fn local(&self) -> &str {
        self.qualified.local()
    }

    /// Returns the exact qualified-name span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.qualified.span()
    }

    pub(super) fn same_qualified_name(&self, other: &Self) -> bool {
        self.qualified.same_name(&other.qualified)
    }

    pub(super) fn display_name(&self) -> Box<str> {
        self.to_string().into_boxed_str()
    }
}

impl fmt::Display for ElementName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.qualified.fmt(formatter)
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

    /// Returns the authored qualified attribute name.
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

/// Case-sensitive qualified name of an attribute.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AttributeName {
    qualified: QualifiedName,
}

impl AttributeName {
    pub(super) fn new(prefix: Option<Box<str>>, local: Box<str>, span: SourceSpan) -> Self {
        Self {
            qualified: QualifiedName::new(prefix, local, span),
        }
    }

    /// Returns the optional authored prefix without resolving namespaces.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.qualified.prefix()
    }

    /// Returns the authored local name.
    #[must_use]
    pub fn local(&self) -> &str {
        self.qualified.local()
    }

    /// Returns the exact qualified-name span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.qualified.span()
    }

    pub(super) fn same_qualified_name(&self, other: &Self) -> bool {
        self.qualified.same_name(&other.qualified)
    }

    pub(super) fn display_name(&self) -> Box<str> {
        self.to_string().into_boxed_str()
    }
}

impl fmt::Display for AttributeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.qualified.fmt(formatter)
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
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct QualifiedName {
    prefix: Option<Box<str>>,
    local: Box<str>,
    span: SourceSpan,
}

impl QualifiedName {
    fn new(prefix: Option<Box<str>>, local: Box<str>, span: SourceSpan) -> Self {
        Self {
            prefix,
            local,
            span,
        }
    }

    fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    fn local(&self) -> &str {
        &self.local
    }

    const fn span(&self) -> SourceSpan {
        self.span
    }

    fn same_name(&self, other: &Self) -> bool {
        self.prefix == other.prefix && self.local == other.local
    }
}

impl fmt::Display for QualifiedName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.prefix {
            write!(formatter, "{prefix}:")?;
        }

        formatter.write_str(&self.local)
    }
}
