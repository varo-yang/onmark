//! Ownership-preserving construction of recovered syntax trees.

use crate::model::{ByteOffset, SourceId, SourceSpan};

use super::{Attribute, Element, ElementName, Node, SourceDocument, SyntaxError, SyntaxErrorKind};

/// Owns the only mutable tree state. The tokenizer adapter recognizes input;
/// this type only enforces nesting ownership and recovery.
pub(super) struct TreeBuilder {
    roots: Vec<Node>,
    stack: Vec<OpenElement>,
    pending: Option<PendingElement>,
}

impl TreeBuilder {
    pub(super) const fn new() -> Self {
        Self {
            roots: Vec::new(),
            stack: Vec::new(),
            pending: None,
        }
    }

    pub(super) fn start_element(&mut self, name: ElementName, start: ByteOffset) {
        self.pending = Some(PendingElement {
            name,
            attributes: Vec::new(),
            start,
        });
    }

    pub(super) fn add_attribute(&mut self, attribute: Attribute) -> Option<SyntaxError> {
        let pending = self
            .pending
            .as_mut()
            .expect("xmlparser emits attributes only after an element start");
        let duplicate = pending
            .attributes
            .iter()
            .find(|current| current.name().same_qualified_name(attribute.name()))
            .map(|first| duplicate_attribute(&attribute, first));

        pending.attributes.push(attribute);
        duplicate
    }

    pub(super) fn open_element(&mut self) {
        let pending = self.take_pending();
        self.stack.push(OpenElement::from(pending));
    }

    pub(super) fn finish_empty_element(&mut self, source: SourceId, end: ByteOffset) {
        let pending = self.take_pending();
        let span = element_span(source, pending.start, end);
        let element = Element::new(pending.name, pending.attributes, Vec::new(), span);
        self.append(Node::Element(element));
    }

    pub(super) fn close_element(
        &mut self,
        source: SourceId,
        found: &ElementName,
        end: ByteOffset,
    ) -> Option<SyntaxError> {
        let Some(open) = self.stack.last() else {
            return Some(SyntaxError::new(
                SyntaxErrorKind::UnexpectedClosingTag {
                    found: found.display_name(),
                },
                found.span(),
            ));
        };

        if !open.name.same_qualified_name(found) {
            // Ignore the mismatched close. Keeping the opener on the stack
            // preserves the trustworthy prefix for EOF recovery.
            return Some(mismatched_closing_tag(open, found));
        }

        self.finish_top_element(source, end);
        None
    }

    pub(super) fn unclosed_elements(&self, ended_at: SourceSpan) -> Vec<SyntaxError> {
        let open = self.stack.iter().map(|element| &element.name);
        let pending = self.pending.iter().map(|element| &element.name);

        open.chain(pending)
            .map(|name| unclosed_element(name, ended_at))
            .collect()
    }

    pub(super) fn finish(&mut self, source: SourceId, end: ByteOffset) {
        self.pending = None;

        // Recovered elements end at EOF. The syntax error records that no
        // authored close established the recovered extent.
        while !self.stack.is_empty() {
            self.finish_top_element(source, end);
        }
    }

    pub(super) fn append(&mut self, node: Node) {
        match self.stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => self.roots.push(node),
        }
    }

    pub(super) fn into_document(self) -> SourceDocument {
        SourceDocument::new(self.roots)
    }

    fn take_pending(&mut self) -> PendingElement {
        self.pending
            .take()
            .expect("xmlparser ends only the current pending element")
    }

    fn finish_top_element(&mut self, source: SourceId, end: ByteOffset) {
        let open = self
            .stack
            .pop()
            .expect("a non-empty stack has a current element");
        let span = element_span(source, open.start, end);
        let element = Element::new(open.name, open.attributes, open.children, span);
        self.append(Node::Element(element));
    }
}

/// Exists only between an element-start token and its matching start-tag end.
struct PendingElement {
    name: ElementName,
    attributes: Vec<Attribute>,
    start: ByteOffset,
}

/// Owns an incomplete subtree until a matching closing token arrives.
struct OpenElement {
    name: ElementName,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
    start: ByteOffset,
}

impl From<PendingElement> for OpenElement {
    fn from(pending: PendingElement) -> Self {
        Self {
            name: pending.name,
            attributes: pending.attributes,
            children: Vec::new(),
            start: pending.start,
        }
    }
}

fn duplicate_attribute(attribute: &Attribute, first: &Attribute) -> SyntaxError {
    SyntaxError::new(
        SyntaxErrorKind::DuplicateAttribute {
            name: attribute.name().display_name(),
            first_declared_at: first.name().span(),
        },
        attribute.name().span(),
    )
}

fn mismatched_closing_tag(open: &OpenElement, found: &ElementName) -> SyntaxError {
    SyntaxError::new(
        SyntaxErrorKind::MismatchedClosingTag {
            expected: open.name.display_name(),
            found: found.display_name(),
            opened_at: open.name.span(),
        },
        found.span(),
    )
}

fn unclosed_element(name: &ElementName, ended_at: SourceSpan) -> SyntaxError {
    SyntaxError::new(
        SyntaxErrorKind::UnclosedElement {
            name: name.display_name(),
            ended_at,
        },
        name.span(),
    )
}

fn element_span(source: SourceId, start: ByteOffset, end: ByteOffset) -> SourceSpan {
    SourceSpan::new(source, start, end).expect("tree-builder spans have ordered bounds")
}
