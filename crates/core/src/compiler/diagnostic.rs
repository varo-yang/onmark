//! Validated construction shared by compiler-owned diagnostic translations.

use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::model::SourceSpan;

pub(super) fn author_diagnostic(
    code: DiagnosticCode,
    primary: SourceSpan,
    message: impl Into<Box<str>>,
    help: impl Into<Box<str>>,
) -> Diagnostic {
    Diagnostic::new(code, primary, message)
        .and_then(|diagnostic| diagnostic.with_help(help))
        .expect("compiler diagnostic messages and repair suggestions are non-blank")
}
