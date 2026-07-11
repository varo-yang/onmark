//! Bounded decoding for the references admitted by the screenplay language.
//!
//! Invalid spellings are retained in recovered text, so diagnostics do not
//! destroy authored input and decoded output never grows beyond source bytes.

use crate::model::{ByteOffset, SourceId, SourceSpan};

use super::{SyntaxError, SyntaxErrorKind};

pub(super) fn decode(
    source: SourceId,
    raw: &str,
    value_start: ByteOffset,
) -> (Box<str>, Vec<SyntaxError>) {
    if !raw.contains('&') {
        return (Box::from(raw), Vec::new());
    }

    let mut decoded = String::with_capacity(raw.len());
    let mut errors = Vec::new();
    let mut cursor = 0;

    while let Some(relative_ampersand) = raw[cursor..].find('&') {
        let ampersand = cursor + relative_ampersand;
        decoded.push_str(&raw[cursor..ampersand]);

        let end = reference_end(raw, ampersand);
        let authored = &raw[ampersand..end];

        if let Some(character) = decode_one(authored) {
            decoded.push(character);
        } else {
            decoded.push_str(authored);
            errors.push(invalid_reference(
                source,
                value_start,
                ampersand,
                end,
                authored,
            ));
        }

        cursor = end;
    }

    if cursor < raw.len() {
        decoded.push_str(&raw[cursor..]);
    }

    (decoded.into_boxed_str(), errors)
}

fn decode_one(reference: &str) -> Option<char> {
    let body = reference.strip_prefix('&')?.strip_suffix(';')?;

    match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => decode_numeric(body),
    }
}

fn reference_end(raw: &str, start: usize) -> usize {
    let body_start = start + 1;

    for (offset, character) in raw[body_start..].char_indices() {
        let position = body_start + offset;

        if character == ';' {
            return position + 1;
        }

        if character == '&' || character.is_ascii_whitespace() {
            return position;
        }
    }

    raw.len()
}

fn decode_numeric(body: &str) -> Option<char> {
    let value = if let Some(hexadecimal) = body.strip_prefix("#x") {
        u32::from_str_radix(hexadecimal, 16).ok()?
    } else {
        body.strip_prefix('#')?.parse::<u32>().ok()?
    };

    if !is_xml_character(value) {
        return None;
    }

    char::from_u32(value)
}

const fn is_xml_character(value: u32) -> bool {
    matches!(
        value,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x0001_0000..=0x0010_FFFF
    )
}

fn invalid_reference(
    source: SourceId,
    value_start: ByteOffset,
    start: usize,
    end: usize,
    authored: &str,
) -> SyntaxError {
    let start = source_offset(value_start, start);
    let end = source_offset(value_start, end);
    let span = SourceSpan::new(source, ByteOffset::new(start), ByteOffset::new(end))
        .expect("a substring has ordered source bounds");

    SyntaxError::new(
        SyntaxErrorKind::InvalidCharacterReference {
            reference: Box::from(authored),
        },
        span,
    )
}

fn source_offset(base: ByteOffset, relative: usize) -> u64 {
    let relative = u64::try_from(relative).expect("Onmark source offsets fit in u64");
    base.get()
        .checked_add(relative)
        .expect("a substring offset fits inside its source")
}

#[cfg(test)]
mod tests {
    use crate::model::{ByteOffset, SourceId};

    use super::decode;

    #[test]
    fn decodes_predefined_and_numeric_references() {
        let (decoded, errors) =
            decode(SourceId::new(0), "&lt;&#65;&#x1F600;&gt;", ByteOffset::ZERO);

        assert!(errors.is_empty());
        assert_eq!(&*decoded, "<A😀>");
    }

    #[test]
    fn preserves_an_invalid_unterminated_reference_once() {
        let (decoded, errors) = decode(SourceId::new(0), "before &bogus", ByteOffset::new(10));

        assert_eq!(&*decoded, "before &bogus");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].span().start().get(), 17);
        assert_eq!(errors[0].span().end().get(), 23);
    }

    #[test]
    fn stops_an_invalid_reference_before_whitespace_or_another_reference() {
        let (decoded, errors) = decode(SourceId::new(0), "&amp&lt; & text", ByteOffset::ZERO);

        assert_eq!(&*decoded, "&amp< & text");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].span().start().get(), 0);
        assert_eq!(errors[0].span().end().get(), 4);
        assert_eq!(errors[1].span().start().get(), 9);
        assert_eq!(errors[1].span().end().get(), 10);
    }
}
