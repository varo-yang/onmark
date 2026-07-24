//! Linear composition of the pure compiler phases for CLI callers.
//!
//! Authored diagnostics accumulate across safe phase boundaries. A phase with
//! errors withholds its value so later phases never consume invalid state.

use std::collections::BTreeMap;

use onmark_core::compiler::{self, ResolvedFilm, SolveError};
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::{AssetRef, FrozenAsset, SourceId, Timebase};
use onmark_core::timeline::TimelineIr;

/// Optional phase value paired with every authored diagnostic collected so far.
#[derive(Debug)]
pub(super) struct Compilation<T> {
    value: Option<T>,
    diagnostics: Vec<Diagnostic>,
}

impl<T> Compilation<T> {
    pub(super) fn rejected(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: None,
            diagnostics,
        }
    }

    pub(super) fn ready(value: T, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: Some(value),
            diagnostics,
        }
    }

    pub(super) fn into_parts(self) -> (Option<T>, Vec<Diagnostic>) {
        (self.value, self.diagnostics)
    }
}

pub(super) fn resolve(source: &str) -> Compilation<ResolvedFilm> {
    let parsed = compiler::parse(SourceId::new(0), source);
    let (document, syntax_diagnostics) = parsed.into_parts();
    let syntax_failed = syntax_diagnostics.has_errors();
    let mut diagnostics = syntax_diagnostics.into_vec();
    if syntax_failed {
        return Compilation::rejected(diagnostics);
    }

    let bound = compiler::bind(document);
    let (film, binding_diagnostics) = bound.into_parts();
    let binding_failed = binding_diagnostics.has_errors();
    diagnostics.extend(binding_diagnostics.into_vec());
    if binding_failed {
        return Compilation::rejected(diagnostics);
    }
    let film = film.expect("binding without error diagnostics produces a linked film");

    let resolved = compiler::resolve(film);
    let (film, resolution_diagnostics) = resolved.into_parts();
    let resolution_failed = resolution_diagnostics.has_errors();
    diagnostics.extend(resolution_diagnostics.into_vec());
    if resolution_failed {
        return Compilation::rejected(diagnostics);
    }
    let film = film.expect("resolution without error diagnostics produces a resolved film");

    Compilation::ready(film, diagnostics)
}

pub(super) fn solve(
    film: ResolvedFilm,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
    timebase: Timebase,
    mut diagnostics: Vec<Diagnostic>,
) -> Result<Compilation<TimelineIr>, SolveError> {
    let solved = compiler::solve(film, assets, timebase)?;
    let (timeline, timing_diagnostics) = solved.into_parts();
    let timing_failed = timing_diagnostics.has_errors();
    diagnostics.extend(timing_diagnostics.into_vec());
    if timing_failed {
        return Ok(Compilation::rejected(diagnostics));
    }
    let timeline = timeline.expect("solving without error diagnostics produces Timeline IR");

    Ok(Compilation::ready(timeline, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn withholds_later_phases_after_authored_errors() {
        let compilation =
            resolve("<om-film><om-scene><om-unknown></om-unknown></om-scene></om-film>");
        let (film, diagnostics) = compilation.into_parts();

        assert!(film.is_none());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code().as_str(), "ONM-STRUCT-001");
    }
}
