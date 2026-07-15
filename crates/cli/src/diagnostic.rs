//! Stable terminal rendering of authored diagnostics against one source file.
//!
//! Compiler spans remain byte-based; this boundary alone projects them into
//! human-readable line and column locations.

use std::io::{self, Write};
use std::path::Path;

use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::{ByteOffset, SourceSpan};

pub(super) fn write_all(
    writer: &mut impl Write,
    source_path: &Path,
    source: &str,
    diagnostics: &[Diagnostic],
) -> io::Result<()> {
    for diagnostic in diagnostics {
        write_one(writer, source_path, source, diagnostic)?;
    }
    Ok(())
}

fn write_one(
    writer: &mut impl Write,
    source_path: &Path,
    source: &str,
    diagnostic: &Diagnostic,
) -> io::Result<()> {
    let location = location(source, diagnostic.primary().start());
    writeln!(
        writer,
        "{}[{}] {}:{}:{}: {}",
        diagnostic.severity(),
        diagnostic.code(),
        source_path.display(),
        location.line,
        location.column,
        diagnostic.message(),
    )?;
    if let Some(help) = diagnostic.help() {
        writeln!(writer, "  help: {help}")?;
    }
    for related in diagnostic.related() {
        write_related(
            writer,
            source_path,
            source,
            related.span(),
            related.message(),
        )?;
    }
    Ok(())
}

fn write_related(
    writer: &mut impl Write,
    source_path: &Path,
    source: &str,
    span: SourceSpan,
    message: &str,
) -> io::Result<()> {
    let location = location(source, span.start());
    writeln!(
        writer,
        "  related: {}:{}:{}: {message}",
        source_path.display(),
        location.line,
        location.column,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Location {
    line: usize,
    column: usize,
}

fn location(source: &str, offset: ByteOffset) -> Location {
    let offset = usize::try_from(offset.get())
        .ok()
        .filter(|offset| *offset <= source.len() && source.is_char_boundary(*offset))
        .unwrap_or(source.len());
    let prefix = &source[..offset];
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);

    Location {
        line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
        column: source[line_start..offset].chars().count() + 1,
    }
}

#[cfg(test)]
mod tests {
    use onmark_core::diagnostics::{Diagnostic, DiagnosticCode};
    use onmark_core::model::{ByteOffset, SourceId, SourceSpan};

    use super::write_all;

    #[test]
    fn renders_utf8_source_locations_in_characters() {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::new(8), ByteOffset::new(8))
            .expect("the fixture span is ordered");
        let diagnostic = Diagnostic::new(DiagnosticCode::UnknownElement, span, "unknown")
            .expect("the fixture message is valid");
        let mut output = Vec::new();

        write_all(
            &mut output,
            std::path::Path::new("film.onmark"),
            "é\n  <x>",
            &[diagnostic],
        )
        .expect("the diagnostic is writable");

        assert_eq!(
            String::from_utf8(output).expect("the output is UTF-8"),
            "error[ONM-STRUCT-001] film.onmark:2:6: unknown\n",
        );
    }
}
