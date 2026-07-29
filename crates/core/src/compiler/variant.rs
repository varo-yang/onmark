//! Typed variant facts carried between compiler phases.
//!
//! Source spellings remain private to binding and resolution. Downstream phases
//! receive only canonical values and explicit semantic dependency scopes.

use std::collections::BTreeMap;

use crate::model::{SourceSpan, VariantFieldKind, VariantFieldName, VariantValue};
use crate::syntax::Attribute;

/// Raw optional schema container retained by structural binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedVariantSchema {
    attributes: Vec<Attribute>,
    span: SourceSpan,
    fields: Vec<LinkedVariantField>,
}

impl LinkedVariantSchema {
    pub(super) const fn new(
        attributes: Vec<Attribute>,
        span: SourceSpan,
        fields: Vec<LinkedVariantField>,
    ) -> Self {
        Self {
            attributes,
            span,
            fields,
        }
    }

    /// Returns authored container attributes in source order.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// Returns the complete authored schema-container span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    /// Returns raw field declarations in authored order.
    #[must_use]
    pub fn fields(&self) -> &[LinkedVariantField] {
        &self.fields
    }

    pub(super) fn into_parts(self) -> (Vec<Attribute>, SourceSpan, Vec<LinkedVariantField>) {
        (self.attributes, self.span, self.fields)
    }
}

/// One field declaration before its attributes are parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedVariantField {
    attributes: Vec<Attribute>,
    span: SourceSpan,
}

impl LinkedVariantField {
    pub(super) const fn new(attributes: Vec<Attribute>, span: SourceSpan) -> Self {
        Self { attributes, span }
    }

    /// Returns authored declaration attributes in source order.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// Returns the complete authored field-declaration span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (Vec<Attribute>, SourceSpan) {
        (self.attributes, self.span)
    }
}

/// Literal DOM operation selected by one binding attribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VariantBindingSink {
    /// Replace direct element text through `textContent`.
    Text,
    /// Set one inline CSS custom property.
    Css,
    /// Set the element's `hidden` property.
    Show,
}

impl VariantBindingSink {
    /// Returns the stable authored attribute spelling.
    #[must_use]
    pub const fn attribute_name(self) -> &'static str {
        match self {
            Self::Text => "data-om-text",
            Self::Css => "data-om-css",
            Self::Show => "data-om-show",
        }
    }
}

/// Semantic owner used to derive exact Render Graph dependencies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkedVariantScope {
    /// Presentation shell owned by the complete film.
    Film,
    /// Presentation shell owned by one scene.
    Scene(SourceSpan),
    /// Presentation state owned by one shot.
    Shot(SourceSpan),
    /// Presentation state owned by one transition boundary.
    Transition(SourceSpan),
}

/// Authored fallback facts needed to prove a truthful static document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LinkedVariantFallback {
    Text {
        value: Box<str>,
        direct_text_only: bool,
    },
    Css {
        style: Option<Attribute>,
    },
    Show {
        hidden: bool,
    },
}

/// One source-located presentation binding before field lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedVariantBinding {
    sink: VariantBindingSink,
    attribute: Attribute,
    fallback: LinkedVariantFallback,
    scope: LinkedVariantScope,
    element_span: SourceSpan,
}

impl LinkedVariantBinding {
    pub(super) const fn new(
        sink: VariantBindingSink,
        attribute: Attribute,
        fallback: LinkedVariantFallback,
        scope: LinkedVariantScope,
        element_span: SourceSpan,
    ) -> Self {
        Self {
            sink,
            attribute,
            fallback,
            scope,
            element_span,
        }
    }

    /// Returns the literal sink selected by this binding.
    #[must_use]
    pub const fn sink(&self) -> VariantBindingSink {
        self.sink
    }

    /// Returns the source attribute that names the field or fields.
    #[must_use]
    pub const fn attribute(&self) -> &Attribute {
        &self.attribute
    }

    /// Returns the semantic dependency owner.
    #[must_use]
    pub const fn scope(&self) -> LinkedVariantScope {
        self.scope
    }

    /// Returns the complete target-element span.
    #[must_use]
    pub const fn element_span(&self) -> SourceSpan {
        self.element_span
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        VariantBindingSink,
        Attribute,
        LinkedVariantFallback,
        LinkedVariantScope,
        SourceSpan,
    ) {
        (
            self.sink,
            self.attribute,
            self.fallback,
            self.scope,
            self.element_span,
        )
    }
}

/// One parsed field declaration with its source identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedVariantField {
    name: VariantFieldName,
    kind: VariantFieldKind,
    default: VariantValue,
    declared_at: SourceSpan,
}

impl ResolvedVariantField {
    pub(super) const fn new(
        name: VariantFieldName,
        kind: VariantFieldKind,
        default: VariantValue,
        declared_at: SourceSpan,
    ) -> Self {
        Self {
            name,
            kind,
            default,
            declared_at,
        }
    }

    /// Returns the film-local field identity.
    #[must_use]
    pub const fn name(&self) -> &VariantFieldName {
        &self.name
    }

    /// Returns the field's closed value kind.
    #[must_use]
    pub const fn kind(&self) -> VariantFieldKind {
        self.kind
    }

    /// Returns the canonical authored default.
    #[must_use]
    pub const fn default(&self) -> &VariantValue {
        &self.default
    }

    /// Returns the source span declaring the field name.
    #[must_use]
    pub const fn declared_at(&self) -> SourceSpan {
        self.declared_at
    }
}

/// One validated field-to-presentation dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedVariantBinding {
    field: VariantFieldName,
    sink: VariantBindingSink,
    scope: LinkedVariantScope,
    authored_at: SourceSpan,
}

impl ResolvedVariantBinding {
    pub(super) const fn new(
        field: VariantFieldName,
        sink: VariantBindingSink,
        scope: LinkedVariantScope,
        authored_at: SourceSpan,
    ) -> Self {
        Self {
            field,
            sink,
            scope,
            authored_at,
        }
    }

    /// Returns the bound field.
    #[must_use]
    pub const fn field(&self) -> &VariantFieldName {
        &self.field
    }

    /// Returns the literal DOM operation.
    #[must_use]
    pub const fn sink(&self) -> VariantBindingSink {
        self.sink
    }

    /// Returns the semantic dependency owner.
    #[must_use]
    pub const fn scope(&self) -> LinkedVariantScope {
        self.scope
    }

    /// Returns the binding value span.
    #[must_use]
    pub const fn authored_at(&self) -> SourceSpan {
        self.authored_at
    }
}

/// Complete typed schema and source-level presentation dependencies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedVariantSchema {
    fields: BTreeMap<VariantFieldName, ResolvedVariantField>,
    bindings: Vec<ResolvedVariantBinding>,
}

impl ResolvedVariantSchema {
    pub(super) const fn new(
        fields: BTreeMap<VariantFieldName, ResolvedVariantField>,
        bindings: Vec<ResolvedVariantBinding>,
    ) -> Self {
        Self { fields, bindings }
    }

    /// Returns fields in canonical name order.
    #[must_use]
    pub fn fields(
        &self,
    ) -> impl ExactSizeIterator<Item = (&VariantFieldName, &ResolvedVariantField)> {
        self.fields.iter()
    }

    /// Returns validated bindings in authored order.
    #[must_use]
    pub fn bindings(&self) -> &[ResolvedVariantBinding] {
        &self.bindings
    }

    /// Finds one declaration by canonical name.
    #[must_use]
    pub fn field(&self, name: &VariantFieldName) -> Option<&ResolvedVariantField> {
        self.fields.get(name)
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        BTreeMap<VariantFieldName, ResolvedVariantField>,
        Vec<ResolvedVariantBinding>,
    ) {
        (self.fields, self.bindings)
    }

    pub(super) fn default_values(&self) -> ResolvedVariantValues {
        ResolvedVariantValues::new(
            self.fields
                .iter()
                .map(|(name, field)| (name.clone(), field.default().clone()))
                .collect(),
        )
    }
}

/// Effective canonical values for one immutable render.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedVariantValues(BTreeMap<VariantFieldName, VariantValue>);

impl ResolvedVariantValues {
    pub(super) const fn new(values: BTreeMap<VariantFieldName, VariantValue>) -> Self {
        Self(values)
    }

    /// Returns effective values in canonical name order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&VariantFieldName, &VariantValue)> {
        self.0.iter()
    }

    /// Finds one effective value.
    #[must_use]
    pub fn get(&self, name: &VariantFieldName) -> Option<&VariantValue> {
        self.0.get(name)
    }

    pub(super) fn into_values(self) -> BTreeMap<VariantFieldName, VariantValue> {
        self.0
    }
}
