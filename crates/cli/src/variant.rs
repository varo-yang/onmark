//! Bounded CLI ingestion for one immutable typed-variant document.
//!
//! The pure compiler owns JSON semantics and source spans; this boundary owns
//! only filesystem access and the path paired with rejected diagnostics.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use onmark_core::compiler::{self, ResolvedFilm};
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::SourceId;

use crate::input::{self, BoundedReadError};

const VARIANT_SOURCE_ID: SourceId = SourceId::new(2);

pub(super) enum VariantImport {
    Film(ResolvedFilm),
    Rejected(RejectedVariant),
}

impl VariantImport {
    pub(super) fn apply(path: Option<&Path>, film: ResolvedFilm) -> Result<Self, VariantLoadError> {
        let Some(path) = path else {
            return Ok(Self::Film(film));
        };
        let source = input::read_utf8(
            path,
            u64::try_from(compiler::MAX_VARIANT_DOCUMENT_BYTES)
                .expect("the variant document limit fits in u64"),
        )
        .map_err(|source| VariantLoadError {
            path: path.to_owned(),
            source,
        })?;
        let report = compiler::resolve_variant(film, VARIANT_SOURCE_ID, &source);
        let (film, diagnostics) = report.into_parts();
        if let Some(film) = film {
            debug_assert!(
                diagnostics.is_empty(),
                "accepted variant input currently has no warning diagnostics",
            );
            return Ok(Self::Film(film));
        }

        Ok(Self::Rejected(RejectedVariant {
            path: path.to_owned(),
            source,
            diagnostics: diagnostics.into_vec(),
        }))
    }
}

pub(super) struct RejectedVariant {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
}

impl RejectedVariant {
    pub(super) fn into_parts(self) -> (PathBuf, String, Vec<Diagnostic>) {
        (self.path, self.source, self.diagnostics)
    }
}

#[derive(Debug)]
pub(super) struct VariantLoadError {
    path: PathBuf,
    source: BoundedReadError,
}

impl fmt::Display for VariantLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to read variant document {}",
            self.path.display(),
        )
    }
}

impl Error for VariantLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use onmark_core::compiler;
    use onmark_core::model::SourceId;
    use tempfile::tempdir;

    use super::VariantImport;

    #[test]
    fn applies_valid_values_and_retains_invalid_json_as_authored_diagnostics() {
        let directory = tempdir().expect("the fixture directory is available");
        let valid = directory.path().join("valid.json");
        let invalid = directory.path().join("invalid.json");
        fs::write(&valid, r##"{"accent":"#112233"}"##).expect("the valid fixture is writable");
        fs::write(&invalid, r#"{"accent":"red"}"#).expect("the invalid fixture is writable");

        assert!(matches!(
            VariantImport::apply(Some(&valid), film()),
            Ok(VariantImport::Film(_)),
        ));
        let rejected = VariantImport::apply(Some(&invalid), film())
            .expect("authored errors remain product output");
        let VariantImport::Rejected(rejected) = rejected else {
            panic!("invalid input must be rejected");
        };
        let (_, _, diagnostics) = rejected.into_parts();
        assert_eq!(diagnostics[0].code().as_str(), "ONM-VARIANT-008");
    }

    fn film() -> compiler::ResolvedFilm {
        let source = r##"
<om-film>
  <om-fields>
    <om-field name="accent" type="color" default="#ff4d36"></om-field>
  </om-fields>
  <om-scene>
    <om-shot duration="1s" data-om-css="accent" style="--accent:#ff4d36"></om-shot>
  </om-scene>
</om-film>
"##;
        let (document, diagnostics) = compiler::parse(SourceId::new(0), source).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
        assert!(diagnostics.is_empty());
        film.expect("the fixture resolves")
    }
}
