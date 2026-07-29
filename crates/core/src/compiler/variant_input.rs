//! Bounded flat-JSON ingestion for one immutable typed variant.
//!
//! A dedicated reader preserves duplicate keys and exact source spans. Generic
//! deserialization into a map would discard both facts before diagnostics own
//! them.

use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{
    ByteOffset, SourceId, SourceSpan, VariantFieldKind, VariantFieldName, VariantValue,
};

use super::diagnostic::author_diagnostic;
use super::resolved_film::ResolvedFilm;
use super::variant::{ResolvedVariantSchema, ResolvedVariantValues};

/// Largest external variant document accepted by the compiler.
pub const MAX_VARIANT_DOCUMENT_BYTES: usize = 1024 * 1024;

/// Optional canonical values and every authored variant-input diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantReport {
    film: Option<ResolvedFilm>,
    diagnostics: Diagnostics,
}

impl VariantReport {
    /// Returns the film with canonical effective values when input is valid.
    #[must_use]
    pub const fn film(&self) -> Option<&ResolvedFilm> {
        self.film.as_ref()
    }

    /// Returns source-located external-input diagnostics.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Returns the optional resolved film and diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<ResolvedFilm>, Diagnostics) {
        (self.film, self.diagnostics)
    }
}

/// Resolves one flat JSON document against a film's closed schema.
///
/// Missing keys retain their declared defaults.
#[must_use]
pub fn resolve_variant(film: ResolvedFilm, source: SourceId, document: &str) -> VariantReport {
    if document.len() > MAX_VARIANT_DOCUMENT_BYTES {
        return invalid_document_report(document_limit(source, document.len()));
    }

    let entries = match JsonReader::new(source, document).read() {
        Ok(entries) => entries,
        Err(error) => return invalid_document_report(error.into_diagnostic()),
    };
    let (values, diagnostics) = resolve_entries(film.variants(), entries);
    let film = values.map(|values| film.with_variant_values(values));
    VariantReport { film, diagnostics }
}

fn resolve_entries(
    schema: &ResolvedVariantSchema,
    entries: Vec<JsonEntry>,
) -> (Option<ResolvedVariantValues>, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let mut values = schema.default_values().into_values();
    let mut authored = BTreeMap::new();

    for entry in entries {
        let Ok(name) = VariantFieldName::parse(&entry.name) else {
            diagnostics.push(unknown_field(&entry));
            continue;
        };
        if let Some(first) = authored.insert(name.clone(), entry.name_span) {
            diagnostics.push(duplicate_key(&entry, first));
            continue;
        }
        let Some(field) = schema.field(&name) else {
            diagnostics.push(unknown_field(&entry));
            continue;
        };
        match entry.value.resolve(field.kind()) {
            Ok(value) => {
                values.insert(name, value);
            }
            Err(reason) => diagnostics.push(invalid_value(&entry, field.kind(), &reason)),
        }
    }

    (
        (!diagnostics.has_errors()).then(|| ResolvedVariantValues::new(values)),
        diagnostics,
    )
}

fn invalid_document_report(diagnostic: Diagnostic) -> VariantReport {
    let mut diagnostics = Diagnostics::new();
    diagnostics.push(diagnostic);
    VariantReport {
        film: None,
        diagnostics,
    }
}

struct JsonReader<'a> {
    source: SourceId,
    input: &'a str,
    cursor: usize,
}

impl<'a> JsonReader<'a> {
    const fn new(source: SourceId, input: &'a str) -> Self {
        Self {
            source,
            input,
            cursor: 0,
        }
    }

    fn read(mut self) -> Result<Vec<JsonEntry>, JsonError> {
        self.skip_whitespace();
        self.expect_byte(b'{', "variant document must begin with an object")?;
        self.skip_whitespace();

        let entries = if self.consume_byte(b'}') {
            Vec::new()
        } else {
            self.read_entries()?
        };
        self.skip_whitespace();
        if self.cursor != self.input.len() {
            return Err(self.error_here("variant document has trailing content"));
        }
        Ok(entries)
    }

    fn read_entries(&mut self) -> Result<Vec<JsonEntry>, JsonError> {
        let mut entries = Vec::new();

        loop {
            self.skip_whitespace();
            let (name, name_span) = self.read_string()?;
            self.skip_whitespace();
            self.expect_byte(b':', "variant field name must be followed by ':'")?;
            self.skip_whitespace();
            let (value, value_span) = self.read_scalar()?;
            entries.push(JsonEntry {
                name,
                name_span,
                value,
                value_span,
            });
            self.skip_whitespace();

            if self.consume_byte(b'}') {
                return Ok(entries);
            }
            self.expect_byte(b',', "variant object entries must be separated by ','")?;
        }
    }

    fn read_scalar(&mut self) -> Result<(JsonScalar, SourceSpan), JsonError> {
        let start = self.cursor;
        let scalar = match self.peek_byte() {
            Some(b'"') => JsonScalar::String(self.read_string()?.0),
            Some(b't') => {
                self.expect_literal("true")?;
                JsonScalar::Boolean(true)
            }
            Some(b'f') => {
                self.expect_literal("false")?;
                JsonScalar::Boolean(false)
            }
            Some(b'-' | b'0'..=b'9') => JsonScalar::Number(self.read_number()?),
            Some(b'[' | b'{') => {
                return Err(self.error_here("variant values must be flat JSON scalars"));
            }
            Some(_) => return Err(self.error_here("variant value is not a supported JSON scalar")),
            None => return Err(self.error_here("variant value is missing")),
        };
        Ok((scalar, self.span(start, self.cursor)))
    }

    fn read_string(&mut self) -> Result<(Box<str>, SourceSpan), JsonError> {
        let quoted_start = self.cursor;
        self.expect_byte(b'"', "JSON object keys and text values must be quoted")?;
        let content_start = self.cursor;
        let mut value = String::new();
        let mut segment_start = self.cursor;

        loop {
            let Some(byte) = self.peek_byte() else {
                return Err(self.error_from(quoted_start, "JSON string is not closed"));
            };
            match byte {
                b'"' => {
                    value.push_str(&self.input[segment_start..self.cursor]);
                    let content_end = self.cursor;
                    self.cursor += 1;
                    return Ok((value.into(), self.span(content_start, content_end)));
                }
                b'\\' => {
                    value.push_str(&self.input[segment_start..self.cursor]);
                    self.cursor += 1;
                    self.read_escape(&mut value)?;
                    segment_start = self.cursor;
                }
                0x00..=0x1f => {
                    return Err(self.error_here("JSON string contains an unescaped control byte"));
                }
                _ => self.advance_char(),
            }
        }
    }

    fn read_escape(&mut self, value: &mut String) -> Result<(), JsonError> {
        let Some(byte) = self.peek_byte() else {
            return Err(self.error_here("JSON escape is incomplete"));
        };
        self.cursor += 1;
        match byte {
            b'"' => value.push('"'),
            b'\\' => value.push('\\'),
            b'/' => value.push('/'),
            b'b' => value.push('\u{0008}'),
            b'f' => value.push('\u{000c}'),
            b'n' => value.push('\n'),
            b'r' => value.push('\r'),
            b't' => value.push('\t'),
            b'u' => self.read_unicode_escape(value)?,
            _ => return Err(self.error_from(self.cursor - 1, "JSON escape is invalid")),
        }
        Ok(())
    }

    fn read_unicode_escape(&mut self, value: &mut String) -> Result<(), JsonError> {
        let first = self.read_hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            self.expect_literal("\\u")?;
            let second = self.read_hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(self.error_here("JSON high surrogate lacks a low surrogate"));
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(self.error_here("JSON low surrogate lacks a high surrogate"));
        } else {
            u32::from(first)
        };
        let character = char::from_u32(scalar)
            .ok_or_else(|| self.error_here("JSON Unicode escape is outside the scalar range"))?;
        value.push(character);
        Ok(())
    }

    fn read_hex_quad(&mut self) -> Result<u16, JsonError> {
        let start = self.cursor;
        let end = start
            .checked_add(4)
            .ok_or_else(|| self.error_here("JSON Unicode escape is too short"))?;
        let Some(value) = self.input.get(start..end) else {
            return Err(self.error_here("JSON Unicode escape is too short"));
        };
        if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(self.error_from(start, "JSON Unicode escape contains a non-hex digit"));
        }
        self.cursor = end;
        u16::from_str_radix(value, 16)
            .map_err(|_| self.error_from(start, "JSON Unicode escape is invalid"))
    }

    fn read_number(&mut self) -> Result<Box<str>, JsonError> {
        let start = self.cursor;
        while self
            .peek_byte()
            .is_some_and(|byte| !matches!(byte, b',' | b'}' | b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.cursor += 1;
        }
        let value = &self.input[start..self.cursor];
        if !is_json_number(value) {
            return Err(self.error_from(start, "JSON number spelling is invalid"));
        }
        Ok(value.into())
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), JsonError> {
        if self.input[self.cursor..].starts_with(literal) {
            self.cursor += literal.len();
            Ok(())
        } else {
            Err(self.error_here("JSON literal is invalid"))
        }
    }

    fn expect_byte(&mut self, expected: u8, message: &'static str) -> Result<(), JsonError> {
        if self.consume_byte(expected) {
            Ok(())
        } else {
            Err(self.error_here(message))
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.peek_byte() != Some(expected) {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek_byte()
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.cursor += 1;
        }
    }

    fn advance_char(&mut self) {
        let width = self.input[self.cursor..]
            .chars()
            .next()
            .expect("the caller proved that one UTF-8 scalar remains")
            .len_utf8();
        self.cursor += width;
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.cursor).copied()
    }

    fn error_here(&self, message: &'static str) -> JsonError {
        self.error_from(self.cursor, message)
    }

    fn error_from(&self, start: usize, message: &'static str) -> JsonError {
        let end = self.cursor.max(start).min(self.input.len());
        JsonError {
            span: self.span(start.min(self.input.len()), end),
            message,
        }
    }

    fn span(&self, start: usize, end: usize) -> SourceSpan {
        SourceSpan::new(
            self.source,
            ByteOffset::new(start as u64),
            ByteOffset::new(end as u64),
        )
        .expect("parser cursors are ordered within a 1 MiB document")
    }
}

fn is_json_number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = usize::from(bytes.first() == Some(&b'-'));
    let Some(first) = bytes.get(cursor) else {
        return false;
    };
    if *first == b'0' {
        cursor += 1;
    } else if first.is_ascii_digit() && *first != b'0' {
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
    } else {
        return false;
    }
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == fraction {
            return false;
        }
    }
    if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let exponent = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == exponent {
            return false;
        }
    }
    cursor == bytes.len()
}

struct JsonEntry {
    name: Box<str>,
    name_span: SourceSpan,
    value: JsonScalar,
    value_span: SourceSpan,
}

enum JsonScalar {
    String(Box<str>),
    Number(Box<str>),
    Boolean(bool),
}

impl JsonScalar {
    fn resolve(&self, kind: VariantFieldKind) -> Result<VariantValue, Box<str>> {
        match (kind, self) {
            (VariantFieldKind::Text, Self::String(value)) => {
                VariantValue::text(value).map_err(|reason| reason.to_string().into())
            }
            (VariantFieldKind::Color, Self::String(value)) => {
                VariantValue::parse(kind, value).map_err(|reason| reason.to_string().into())
            }
            (VariantFieldKind::Integer, Self::Number(value)) => {
                VariantValue::parse(kind, value).map_err(|reason| reason.to_string().into())
            }
            (VariantFieldKind::Boolean, Self::Boolean(value)) => Ok(VariantValue::Boolean(*value)),
            (kind, _) => Err(format!("JSON scalar does not match {kind}").into()),
        }
    }
}

#[derive(Debug)]
struct JsonError {
    span: SourceSpan,
    message: &'static str,
}

impl JsonError {
    fn into_diagnostic(self) -> Diagnostic {
        author_diagnostic(
            DiagnosticCode::InvalidVariantDocument,
            self.span,
            self.message,
            "use one flat JSON object containing only declared scalar fields",
        )
    }
}

fn document_limit(source: SourceId, length: usize) -> Diagnostic {
    let end = u64::try_from(length.min(MAX_VARIANT_DOCUMENT_BYTES))
        .expect("the variant document limit fits u64");
    let span = SourceSpan::new(source, ByteOffset::ZERO, ByteOffset::new(end))
        .expect("zero through the bounded prefix is ordered");
    author_diagnostic(
        DiagnosticCode::InvalidVariantDocument,
        span,
        format!("variant document exceeds {MAX_VARIANT_DOCUMENT_BYTES} bytes"),
        "reduce the variant document to at most 1 MiB",
    )
}

fn duplicate_key(entry: &JsonEntry, first: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantDocument,
        entry.name_span,
        format!("variant field \"{}\" appears more than once", entry.name),
        "keep one value for this field",
    )
    .with_related(first, "the first key is here")
    .expect("the static related message is non-blank")
}

fn unknown_field(entry: &JsonEntry) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownVariantField,
        entry.name_span,
        format!("variant document names undeclared field \"{}\"", entry.name),
        "remove this key or declare the field in <om-fields>",
    )
}

fn invalid_value(entry: &JsonEntry, kind: VariantFieldKind, reason: &str) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidVariantValue,
        entry.value_span,
        format!(
            "value for variant field \"{}\" is not valid {kind}: {reason}",
            entry.name,
        ),
        format!("provide one canonical {kind} value"),
    )
}

#[cfg(test)]
mod tests {
    use super::{JsonReader, JsonScalar, is_json_number};
    use crate::model::SourceId;

    #[test]
    fn reads_flat_scalars_and_unicode_escapes() {
        let entries = JsonReader::new(
            SourceId::new(1),
            r#"{"headline":"A \uD83D\uDE80","progress":72,"featured":false}"#,
        )
        .read()
        .expect("the flat document is valid");

        assert_eq!(entries.len(), 3);
        assert_eq!(&*entries[0].name, "headline");
        assert!(matches!(
            &entries[0].value,
            JsonScalar::String(value) if &**value == "A 🚀"
        ));
    }

    #[test]
    fn validates_json_number_grammar_before_field_typing() {
        for value in ["0", "-1", "1.5", "1e3", "-2.5E-2"] {
            assert!(is_json_number(value), "{value}");
        }
        for value in ["01", "+1", "1.", ".5", "1e"] {
            assert!(!is_json_number(value), "{value}");
        }
    }
}
