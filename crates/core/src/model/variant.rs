//! Canonical presentation-input values shared across compiler phases.
//!
//! These values can affect pixels but never timing, structure, or resource
//! selection. Construction rejects every spelling that would have more than one
//! wire representation.

use std::error::Error;
use std::fmt;

/// Largest field name admitted by the screenplay language.
pub const MAX_VARIANT_FIELD_NAME_BYTES: usize = 64;

/// Largest UTF-8 text value admitted for one field.
pub const MAX_VARIANT_TEXT_BYTES: usize = 16 * 1024;

/// Largest integer represented exactly by JavaScript and JSON consumers.
pub const MAX_EXACT_VARIANT_INTEGER: i64 = 9_007_199_254_740_991;

/// Film-local identity of one typed presentation input.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VariantFieldName(Box<str>);

impl VariantFieldName {
    /// Parses one lower-camel ASCII field name.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVariantFieldName`] when the name is empty, too long,
    /// does not begin with a lowercase ASCII letter, or contains a non-alphanumeric
    /// byte.
    pub fn parse(value: &str) -> Result<Self, InvalidVariantFieldName> {
        let Some(first) = value.as_bytes().first() else {
            return Err(InvalidVariantFieldName::Empty);
        };
        if value.len() > MAX_VARIANT_FIELD_NAME_BYTES {
            return Err(InvalidVariantFieldName::TooLong);
        }
        if !first.is_ascii_lowercase() {
            return Err(InvalidVariantFieldName::InvalidStart);
        }
        if !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(InvalidVariantFieldName::InvalidCharacter);
        }

        Ok(Self(value.into()))
    }

    /// Returns the canonical field name.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VariantFieldName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reason a field name cannot enter typed compiler state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidVariantFieldName {
    /// The name contains no bytes.
    Empty,
    /// The name exceeds the language bound.
    TooLong,
    /// The first byte is not a lowercase ASCII letter.
    InvalidStart,
    /// A later byte is not an ASCII letter or digit.
    InvalidCharacter,
}

impl fmt::Display for InvalidVariantFieldName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "field name is empty",
            Self::TooLong => "field name exceeds 64 bytes",
            Self::InvalidStart => "field name must begin with a lowercase ASCII letter",
            Self::InvalidCharacter => "field name must contain only ASCII letters and digits",
        })
    }
}

impl Error for InvalidVariantFieldName {}

/// Closed kinds of externally supplied presentation values.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VariantFieldKind {
    /// Decoded Unicode text.
    Text,
    /// Exact signed integer.
    Integer,
    /// Visibility or another binary presentation choice.
    Boolean,
    /// Canonical sRGB hexadecimal color.
    Color,
}

impl VariantFieldKind {
    /// Parses one stable language spelling.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVariantFieldKind`] when the spelling is outside the
    /// closed language vocabulary.
    pub fn parse(value: &str) -> Result<Self, InvalidVariantFieldKind> {
        match value {
            "text" => Ok(Self::Text),
            "integer" => Ok(Self::Integer),
            "boolean" => Ok(Self::Boolean),
            "color" => Ok(Self::Color),
            _ => Err(InvalidVariantFieldKind),
        }
    }

    /// Returns the stable language and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Color => "color",
        }
    }
}

impl fmt::Display for VariantFieldKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A field kind outside the closed language vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidVariantFieldKind;

impl fmt::Display for InvalidVariantFieldKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("field type must be text, integer, boolean, or color")
    }
}

impl Error for InvalidVariantFieldKind {}

/// One canonical presentation value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum VariantValue {
    /// Decoded UTF-8 text.
    Text(Box<str>),
    /// Exact signed integer.
    Integer(i64),
    /// Binary presentation choice.
    Boolean(bool),
    /// Lowercase hexadecimal sRGB color.
    Color(Box<str>),
}

impl VariantValue {
    /// Parses one canonical authored default.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVariantValue`] when the value does not use the selected
    /// kind's unique spelling or exceeds its bounded domain.
    pub fn parse(kind: VariantFieldKind, value: &str) -> Result<Self, InvalidVariantValue> {
        match kind {
            VariantFieldKind::Text => Self::text(value),
            VariantFieldKind::Integer => Self::integer(value),
            VariantFieldKind::Boolean => Self::boolean(value),
            VariantFieldKind::Color => Self::color(value),
        }
    }

    /// Constructs a bounded text value.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVariantValue::TextTooLong`] when the UTF-8 value
    /// exceeds the presentation-input bound.
    pub fn text(value: &str) -> Result<Self, InvalidVariantValue> {
        if value.len() > MAX_VARIANT_TEXT_BYTES {
            return Err(InvalidVariantValue::TextTooLong);
        }
        Ok(Self::Text(value.into()))
    }

    /// Constructs an exact integer value.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVariantValue::IntegerOutOfRange`] when JavaScript
    /// cannot represent the integer exactly.
    pub fn from_integer(value: i64) -> Result<Self, InvalidVariantValue> {
        if value.unsigned_abs() > MAX_EXACT_VARIANT_INTEGER as u64 {
            return Err(InvalidVariantValue::IntegerOutOfRange);
        }
        Ok(Self::Integer(value))
    }

    /// Returns this value's closed kind.
    #[must_use]
    pub const fn kind(&self) -> VariantFieldKind {
        match self {
            Self::Text(_) => VariantFieldKind::Text,
            Self::Integer(_) => VariantFieldKind::Integer,
            Self::Boolean(_) => VariantFieldKind::Boolean,
            Self::Color(_) => VariantFieldKind::Color,
        }
    }

    /// Returns a text value without changing its bytes.
    #[must_use]
    pub const fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Integer(_) | Self::Boolean(_) | Self::Color(_) => None,
        }
    }

    /// Returns an integer value.
    #[must_use]
    pub const fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Text(_) | Self::Boolean(_) | Self::Color(_) => None,
        }
    }

    /// Returns a boolean value.
    #[must_use]
    pub const fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            Self::Text(_) | Self::Integer(_) | Self::Color(_) => None,
        }
    }

    /// Returns a canonical color value.
    #[must_use]
    pub const fn as_color(&self) -> Option<&str> {
        match self {
            Self::Color(value) => Some(value),
            Self::Text(_) | Self::Integer(_) | Self::Boolean(_) => None,
        }
    }

    fn integer(value: &str) -> Result<Self, InvalidVariantValue> {
        if !is_canonical_integer(value) {
            return Err(InvalidVariantValue::InvalidInteger);
        }
        let value = value
            .parse()
            .map_err(|_| InvalidVariantValue::IntegerOutOfRange)?;
        Self::from_integer(value)
    }

    fn boolean(value: &str) -> Result<Self, InvalidVariantValue> {
        match value {
            "true" => Ok(Self::Boolean(true)),
            "false" => Ok(Self::Boolean(false)),
            _ => Err(InvalidVariantValue::InvalidBoolean),
        }
    }

    fn color(value: &str) -> Result<Self, InvalidVariantValue> {
        if !is_canonical_color(value) {
            return Err(InvalidVariantValue::InvalidColor);
        }
        Ok(Self::Color(value.into()))
    }
}

impl fmt::Display for VariantValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(value) | Self::Color(value) => formatter.write_str(value),
            Self::Integer(value) => value.fmt(formatter),
            Self::Boolean(value) => value.fmt(formatter),
        }
    }
}

fn is_canonical_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && value != "-0"
}

fn is_canonical_color(value: &str) -> bool {
    matches!(value.len(), 7 | 9)
        && value.starts_with('#')
        && value[1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Reason a variant value cannot enter canonical compiler state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidVariantValue {
    /// UTF-8 text exceeds its per-field bound.
    TextTooLong,
    /// Integer spelling is not canonical base ten.
    InvalidInteger,
    /// Integer magnitude exceeds JavaScript's exact range.
    IntegerOutOfRange,
    /// Boolean spelling is not one of the two canonical literals.
    InvalidBoolean,
    /// Color spelling is not lowercase six- or eight-digit hexadecimal.
    InvalidColor,
}

impl fmt::Display for InvalidVariantValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TextTooLong => "text value exceeds 16 KiB of UTF-8",
            Self::InvalidInteger => "integer must use canonical base-ten spelling",
            Self::IntegerOutOfRange => "integer exceeds JavaScript's exact signed range",
            Self::InvalidBoolean => "boolean must be true or false",
            Self::InvalidColor => "color must be lowercase #rrggbb or #rrggbbaa",
        })
    }
}

impl Error for InvalidVariantValue {}

#[cfg(test)]
mod tests {
    use super::{
        InvalidVariantFieldName, InvalidVariantValue, VariantFieldKind, VariantFieldName,
        VariantValue,
    };

    #[test]
    fn field_names_use_one_lower_camel_ascii_domain() {
        assert_eq!(
            VariantFieldName::parse("accent2")
                .expect("a lower-camel ASCII name is valid")
                .as_str(),
            "accent2",
        );
        assert_eq!(
            VariantFieldName::parse("Accent"),
            Err(InvalidVariantFieldName::InvalidStart),
        );
        assert_eq!(
            VariantFieldName::parse("accent-color"),
            Err(InvalidVariantFieldName::InvalidCharacter),
        );
    }

    #[test]
    fn integer_spelling_is_unique_and_exact() {
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Integer, "9007199254740991"),
            Ok(VariantValue::Integer(9_007_199_254_740_991)),
        );
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Integer, "01"),
            Err(InvalidVariantValue::InvalidInteger),
        );
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Integer, "-0"),
            Err(InvalidVariantValue::InvalidInteger),
        );
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Integer, "9007199254740992"),
            Err(InvalidVariantValue::IntegerOutOfRange),
        );
    }

    #[test]
    fn colors_and_booleans_have_one_spelling() {
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Color, "#ff4d36"),
            Ok(VariantValue::Color("#ff4d36".into())),
        );
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Color, "#FF4D36"),
            Err(InvalidVariantValue::InvalidColor),
        );
        assert_eq!(
            VariantValue::parse(VariantFieldKind::Boolean, "false"),
            Ok(VariantValue::Boolean(false)),
        );
    }
}
