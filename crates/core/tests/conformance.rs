//! Stable-text support shared by conformance test targets.

// Integration targets compile this shared module independently and each uses
// only the renderers relevant to its own conformance layer.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use onmark_core::diagnostics::{Diagnostic, Diagnostics};
use onmark_core::model::SourceSpan;

pub(crate) fn fixture(suite: &str, relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance")
        .join(suite)
        .join(relative)
}

pub(crate) fn assert_or_update(path: &Path, actual: &str) {
    if std::env::var_os("ONMARK_UPDATE_GOLDENS").is_some() {
        fs::write(path, actual).expect("the generated golden artifact must be writable");
        return;
    }

    let expected = fs::read_to_string(path).expect(
        "the golden artifact must exist; regenerate with ONMARK_UPDATE_GOLDENS=1 cargo test",
    );
    assert_eq!(actual, expected, "golden mismatch for {}", path.display());
}

pub(crate) fn render_diagnostics(diagnostics: &Diagnostics) -> String {
    let mut renderer = DiagnosticRenderer::new();

    for diagnostic in diagnostics.iter() {
        renderer
            .render(diagnostic)
            .expect("rendering diagnostics into a String cannot fail");
    }

    renderer.finish()
}

pub(crate) fn span(span: SourceSpan) -> String {
    format!(
        "{}:{}..{}",
        span.source().get(),
        span.start().get(),
        span.end().get(),
    )
}

struct DiagnosticRenderer {
    output: String,
}

impl DiagnosticRenderer {
    fn new() -> Self {
        Self {
            output: String::from("# onmark diagnostic test rendering; not a wire format\n"),
        }
    }

    fn render(&mut self, diagnostic: &Diagnostic) -> std::fmt::Result {
        writeln!(
            self.output,
            "{} {} @{} {}",
            diagnostic.code(),
            diagnostic.severity(),
            span(diagnostic.primary()),
            diagnostic.message(),
        )?;

        if let Some(help) = diagnostic.help() {
            writeln!(self.output, "  help: {help}")?;
        }

        for related in diagnostic.related() {
            writeln!(
                self.output,
                "  related @{} {}",
                span(related.span()),
                related.message(),
            )?;
        }

        Ok(())
    }

    fn finish(self) -> String {
        self.output
    }
}
