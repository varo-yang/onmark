//! Resolution of typed field declarations and literal presentation bindings.

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{
    InvalidVariantFieldKind, InvalidVariantFieldName, InvalidVariantValue, SourceSpan,
    VariantFieldKind, VariantFieldName, VariantValue,
};
use crate::syntax::Attribute;

use super::diagnostic::author_diagnostic;
use super::variant::{
    LinkedVariantBinding, LinkedVariantFallback, LinkedVariantField, LinkedVariantSchema,
    ResolvedVariantBinding, ResolvedVariantField, ResolvedVariantSchema, VariantBindingSink,
};

const MAX_VARIANT_FIELDS: usize = 256;

pub(super) fn resolve_variant_schema(
    schema: Option<LinkedVariantSchema>,
    bindings: Vec<LinkedVariantBinding>,
    diagnostics: &mut Diagnostics,
) -> ResolvedVariantSchema {
    let fields = resolve_fields(schema, diagnostics);
    let bindings = resolve_bindings(bindings, &fields, diagnostics);
    report_unused_fields(&fields, &bindings, diagnostics);
    ResolvedVariantSchema::new(fields, bindings)
}

fn resolve_fields(
    schema: Option<LinkedVariantSchema>,
    diagnostics: &mut Diagnostics,
) -> BTreeMap<VariantFieldName, ResolvedVariantField> {
    let Some(schema) = schema else {
        return BTreeMap::new();
    };
    let (attributes, span, declarations) = schema.into_parts();

    for attribute in attributes {
        diagnostics.push(invalid_schema_attribute(&attribute));
    }
    if declarations.len() > MAX_VARIANT_FIELDS {
        diagnostics.push(too_many_fields(span, declarations.len()));
    }

    let mut fields = BTreeMap::new();
    for declaration in declarations.into_iter().take(MAX_VARIANT_FIELDS) {
        let Some(field) = resolve_field(declaration, diagnostics) else {
            continue;
        };
        if let Some(first) = fields.get(field.name()) {
            diagnostics.push(duplicate_field(&field, first));
            continue;
        }
        fields.insert(field.name().clone(), field);
    }
    fields
}

fn resolve_field(
    declaration: LinkedVariantField,
    diagnostics: &mut Diagnostics,
) -> Option<ResolvedVariantField> {
    let (attributes, span) = declaration.into_parts();
    let declared_at = field_name_span(&attributes, span);
    let mut attributes = FieldAttributes::new(attributes);
    let name = attributes.take("name");
    let kind = attributes.take("type");
    let default = attributes.take("default");

    for attribute in attributes.remaining {
        diagnostics.push(invalid_field_attribute(&attribute));
    }

    let name = parse_field_name(name, span, diagnostics);
    let kind = parse_field_kind(kind, span, diagnostics);
    let default = parse_field_default(default, kind.as_ref(), span, diagnostics);

    Some(ResolvedVariantField::new(
        name?,
        kind?,
        default?,
        declared_at,
    ))
}

fn parse_field_name(
    attribute: Option<Attribute>,
    element_span: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<VariantFieldName> {
    let Some(attribute) = attribute else {
        diagnostics.push(missing_field_attribute("name", element_span));
        return None;
    };
    match VariantFieldName::parse(attribute.value()) {
        Ok(name) => Some(name),
        Err(reason) => {
            diagnostics.push(invalid_field_name(&attribute, reason));
            None
        }
    }
}

fn parse_field_kind(
    attribute: Option<Attribute>,
    element_span: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<VariantFieldKind> {
    let Some(attribute) = attribute else {
        diagnostics.push(missing_field_attribute("type", element_span));
        return None;
    };
    match VariantFieldKind::parse(attribute.value()) {
        Ok(kind) => Some(kind),
        Err(reason) => {
            diagnostics.push(invalid_field_kind(&attribute, reason));
            None
        }
    }
}

fn parse_field_default(
    attribute: Option<Attribute>,
    kind: Option<&VariantFieldKind>,
    element_span: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<VariantValue> {
    let Some(attribute) = attribute else {
        diagnostics.push(missing_field_attribute("default", element_span));
        return None;
    };
    let kind = kind?;
    match VariantValue::parse(*kind, attribute.value()) {
        Ok(default) => Some(default),
        Err(reason) => {
            diagnostics.push(invalid_field_default(&attribute, *kind, reason));
            None
        }
    }
}

fn field_name_span(attributes: &[Attribute], fallback: SourceSpan) -> SourceSpan {
    attributes
        .iter()
        .find(|attribute| attribute.name().local() == "name")
        .map_or(fallback, Attribute::value_span)
}

fn resolve_bindings(
    bindings: Vec<LinkedVariantBinding>,
    fields: &BTreeMap<VariantFieldName, ResolvedVariantField>,
    diagnostics: &mut Diagnostics,
) -> Vec<ResolvedVariantBinding> {
    let mut resolved = Vec::new();

    for binding in bindings {
        let (sink, attribute, fallback, scope, element_span) = binding.into_parts();
        let names = binding_names(sink, &attribute, diagnostics);
        let mut seen = BTreeSet::new();

        for name in names {
            if !seen.insert(name.clone()) {
                diagnostics.push(duplicate_binding_name(&attribute, &name));
                continue;
            }
            let Some(field) = fields.get(&name) else {
                diagnostics.push(unknown_binding_field(&attribute, &name));
                continue;
            };
            if !sink_accepts(sink, field.kind()) {
                diagnostics.push(incompatible_binding(&attribute, sink, field));
                continue;
            }
            if !fallback_matches(&fallback, sink, field, element_span, diagnostics) {
                continue;
            }
            resolved.push(ResolvedVariantBinding::new(
                name,
                sink,
                scope,
                attribute.value_span(),
            ));
        }
    }

    resolved
}

fn binding_names(
    sink: VariantBindingSink,
    attribute: &Attribute,
    diagnostics: &mut Diagnostics,
) -> Vec<VariantFieldName> {
    let values = match sink {
        VariantBindingSink::Css => attribute.value().split_ascii_whitespace().collect(),
        VariantBindingSink::Text | VariantBindingSink::Show => vec![attribute.value()],
    };
    if values.is_empty() {
        diagnostics.push(invalid_binding_name(attribute));
        return Vec::new();
    }

    let mut names = Vec::with_capacity(values.len());
    for value in values {
        let Ok(name) = VariantFieldName::parse(value) else {
            diagnostics.push(invalid_binding_name(attribute));
            continue;
        };
        names.push(name);
    }
    names
}

fn sink_accepts(sink: VariantBindingSink, kind: VariantFieldKind) -> bool {
    matches!(
        (sink, kind),
        (VariantBindingSink::Text, VariantFieldKind::Text)
            | (
                VariantBindingSink::Css,
                VariantFieldKind::Color | VariantFieldKind::Integer
            )
            | (VariantBindingSink::Show, VariantFieldKind::Boolean)
    )
}

fn fallback_matches(
    fallback: &LinkedVariantFallback,
    sink: VariantBindingSink,
    field: &ResolvedVariantField,
    element_span: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> bool {
    let matches = match fallback {
        LinkedVariantFallback::Text {
            value,
            direct_text_only,
        } => *direct_text_only && field.default().as_text() == Some(value),
        LinkedVariantFallback::Css { style } => style
            .as_ref()
            .and_then(|style| css_custom_property(style.value(), field.name().as_str()))
            .is_some_and(|value| value == field.default().to_string()),
        LinkedVariantFallback::Show { hidden } => field
            .default()
            .as_boolean()
            .is_some_and(|value| *hidden != value),
    };
    if !matches {
        diagnostics.push(invalid_fallback(field, sink, element_span));
    }
    matches
}

fn css_custom_property<'a>(style: &'a str, field: &str) -> Option<&'a str> {
    let property = format!("--{field}");
    let mut found = None;

    for declaration in CssDeclarations::new(style) {
        let Some((name, value)) = split_declaration(declaration) else {
            continue;
        };
        if name != property {
            continue;
        }
        if found.replace(value).is_some() {
            return None;
        }
    }
    found
}

fn split_declaration(declaration: &str) -> Option<(&str, &str)> {
    let colon = top_level_colon(declaration)?;
    let name = trim_css_trivia(&declaration[..colon]);
    let value = trim_css_trivia(&declaration[colon + 1..]);
    (!name.is_empty() && !value.is_empty()).then_some((name, value))
}

fn trim_css_trivia(mut value: &str) -> &str {
    loop {
        value = value.trim();
        if let Some(comment) = value.strip_prefix("/*") {
            let Some(end) = comment.find("*/") else {
                return value;
            };
            value = &comment[end + 2..];
            continue;
        }
        if let Some(comment) = value.strip_suffix("*/") {
            let Some(start) = comment.rfind("/*") else {
                return value;
            };
            value = &comment[..start];
            continue;
        }
        return value;
    }
}

fn top_level_colon(value: &str) -> Option<usize> {
    CssScanner::new(value)
        .find(|(_, byte, depth)| *byte == b':' && *depth == 0)
        .map(|(index, _, _)| index)
}

struct CssDeclarations<'a> {
    source: &'a str,
    start: usize,
    scanner: CssScanner<'a>,
}

impl<'a> CssDeclarations<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            start: 0,
            scanner: CssScanner::new(source),
        }
    }
}

impl<'a> Iterator for CssDeclarations<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        for (index, byte, depth) in self.scanner.by_ref() {
            if byte == b';' && depth == 0 {
                let declaration = &self.source[self.start..index];
                self.start = index + 1;
                return Some(declaration);
            }
        }

        if self.start > self.source.len() {
            return None;
        }
        let declaration = &self.source[self.start..];
        self.start = self.source.len() + 1;
        Some(declaration)
    }
}

struct CssScanner<'a> {
    bytes: &'a [u8],
    index: usize,
    depth: usize,
    quote: Option<u8>,
    comment: bool,
    escaped: bool,
}

impl<'a> CssScanner<'a> {
    const fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            index: 0,
            depth: 0,
            quote: None,
            comment: false,
            escaped: false,
        }
    }
}

impl Iterator for CssScanner<'_> {
    type Item = (usize, u8, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let byte = *self.bytes.get(self.index)?;
            let index = self.index;
            let depth = self.depth;
            self.index += 1;

            if self.comment {
                if byte == b'*' && self.bytes.get(self.index) == Some(&b'/') {
                    self.index += 1;
                    self.comment = false;
                }
                continue;
            }
            if self.escaped {
                self.escaped = false;
                continue;
            }
            if byte == b'\\' {
                self.escaped = true;
                continue;
            }
            if let Some(quote) = self.quote {
                if byte == quote {
                    self.quote = None;
                }
                continue;
            }
            if matches!(byte, b'\'' | b'"') {
                self.quote = Some(byte);
                continue;
            }
            if byte == b'/' && self.bytes.get(self.index) == Some(&b'*') {
                self.index += 1;
                self.comment = true;
                continue;
            }
            if byte == b'(' {
                self.depth = self.depth.saturating_add(1);
            } else if byte == b')' {
                self.depth = self.depth.saturating_sub(1);
            }

            return Some((index, byte, depth));
        }
    }
}

fn report_unused_fields(
    fields: &BTreeMap<VariantFieldName, ResolvedVariantField>,
    bindings: &[ResolvedVariantBinding],
    diagnostics: &mut Diagnostics,
) {
    let used = bindings
        .iter()
        .map(ResolvedVariantBinding::field)
        .collect::<BTreeSet<_>>();
    for field in fields.values() {
        if !used.contains(field.name()) {
            diagnostics.push(unused_field(field));
        }
    }
}

struct FieldAttributes {
    remaining: Vec<Attribute>,
}

impl FieldAttributes {
    fn new(attributes: Vec<Attribute>) -> Self {
        Self {
            remaining: attributes,
        }
    }

    fn take(&mut self, name: &str) -> Option<Attribute> {
        let index = self
            .remaining
            .iter()
            .position(|attribute| attribute.name().local() == name)?;
        Some(self.remaining.remove(index))
    }
}

fn invalid_schema_attribute(attribute: &Attribute) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        attribute.name().span(),
        format!(
            "attribute \"{}\" is not allowed on <om-fields>",
            attribute.name(),
        ),
        "remove this attribute from <om-fields>",
    )
}

fn too_many_fields(span: SourceSpan, count: usize) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        span,
        format!("film declares {count} variant fields; the limit is {MAX_VARIANT_FIELDS}"),
        "keep at most 256 fields in one <om-fields> container",
    )
}

fn invalid_field_attribute(attribute: &Attribute) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        attribute.name().span(),
        format!(
            "attribute \"{}\" is not allowed on <om-field>",
            attribute.name(),
        ),
        "keep only name, type, and default on <om-field>",
    )
}

fn missing_field_attribute(name: &str, span: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        span,
        format!("<om-field> requires the \"{name}\" attribute"),
        format!("add {name}=\"...\" to this <om-field>"),
    )
}

fn invalid_field_name(attribute: &Attribute, reason: InvalidVariantFieldName) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        attribute.value_span(),
        format!(
            "variant field name \"{}\" is invalid: {reason}",
            attribute.value()
        ),
        "use a lower-camel ASCII name such as headline or accent2",
    )
}

fn invalid_field_kind(attribute: &Attribute, reason: InvalidVariantFieldKind) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        attribute.value_span(),
        format!(
            "variant field type \"{}\" is invalid: {reason}",
            attribute.value()
        ),
        "use text, integer, boolean, or color",
    )
}

fn invalid_field_default(
    attribute: &Attribute,
    kind: VariantFieldKind,
    reason: InvalidVariantValue,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDeclaration,
        attribute.value_span(),
        format!("default for {kind} field is invalid: {reason}"),
        format!("use one canonical {kind} value"),
    )
}

fn duplicate_field(field: &ResolvedVariantField, first: &ResolvedVariantField) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::DuplicateVariantField,
        field.declared_at(),
        format!(
            "variant field \"{}\" is declared more than once",
            field.name()
        ),
        "keep one declaration for this field",
    )
    .with_related(first.declared_at(), "the first declaration is here")
    .expect("the static related message is non-blank")
}

fn invalid_binding_name(attribute: &Attribute) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownVariantFieldBinding,
        attribute.value_span(),
        format!(
            "attribute \"{}\" does not contain a valid field name",
            attribute.name(),
        ),
        "use a declared lower-camel field name",
    )
}

fn duplicate_binding_name(attribute: &Attribute, name: &VariantFieldName) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::IncompatibleVariantBinding,
        attribute.value_span(),
        format!("field \"{name}\" is repeated in this binding"),
        "name each field once in one binding attribute",
    )
}

fn unknown_binding_field(attribute: &Attribute, name: &VariantFieldName) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownVariantFieldBinding,
        attribute.value_span(),
        format!("presentation binding names undeclared field \"{name}\""),
        format!("declare \"{name}\" in <om-fields> or remove this binding"),
    )
}

fn incompatible_binding(
    attribute: &Attribute,
    sink: VariantBindingSink,
    field: &ResolvedVariantField,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::IncompatibleVariantBinding,
        attribute.value_span(),
        format!(
            "{} cannot bind {} field \"{}\"",
            sink.attribute_name(),
            field.kind(),
            field.name(),
        ),
        "bind text through data-om-text, boolean through data-om-show, and color or integer through data-om-css",
    )
}

fn invalid_fallback(
    field: &ResolvedVariantField,
    sink: VariantBindingSink,
    element_span: SourceSpan,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantFallback,
        element_span,
        format!(
            "{} fallback for field \"{}\" does not equal its canonical default",
            sink.attribute_name(),
            field.name(),
        ),
        fallback_help(field, sink),
    )
}

fn fallback_help(field: &ResolvedVariantField, sink: VariantBindingSink) -> String {
    match sink {
        VariantBindingSink::Text => format!(
            "use direct text \"{}\" with no child elements",
            field.default(),
        ),
        VariantBindingSink::Css => format!(
            "initialize --{}:{} in this element's inline style",
            field.name(),
            field.default(),
        ),
        VariantBindingSink::Show => match field.default().as_boolean() {
            Some(true) => "remove hidden because the default is true".to_owned(),
            Some(false) => "add hidden because the default is false".to_owned(),
            None => "bind a boolean field through data-om-show".to_owned(),
        },
    }
}

fn unused_field(field: &ResolvedVariantField) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnusedVariantField,
        field.declared_at(),
        format!(
            "variant field \"{}\" has no presentation binding",
            field.name()
        ),
        "bind this field through data-om-text, data-om-css, or data-om-show, or remove it",
    )
}

#[cfg(test)]
mod tests {
    use super::{css_custom_property, split_declaration};

    #[test]
    fn css_fallback_scanning_respects_strings_and_functions() {
        let style = concat!(
            r#"--label:"a;b";"#,
            r#"background:url("data:image/svg+xml;a:b");"#,
            "/* unrelated ; : declaration */ --accent:#ff4d36;",
            "--progress:72",
        );

        assert_eq!(css_custom_property(style, "accent"), Some("#ff4d36"));
        assert_eq!(css_custom_property(style, "progress"), Some("72"));
        assert_eq!(
            split_declaration("--accent:#ff4d36"),
            Some(("--accent", "#ff4d36"))
        );
    }

    #[test]
    fn duplicate_css_fallbacks_are_ambiguous() {
        let style = "--accent:#ff4d36;--accent:#000000";

        assert_eq!(css_custom_property(style, "accent"), None);
    }
}
