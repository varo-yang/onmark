//! Pure orchestration across screenplay compilation phases.
//!
//! ```
//! use std::collections::BTreeMap;
//!
//! use onmark_core::compiler;
//! use onmark_core::model::{FrameRate, SourceId, Timebase};
//!
//! let source = r#"<film><scene><shot duration="1s" /></scene></film>"#;
//! let parsed = compiler::parse(SourceId::new(0), source);
//! let (document, syntax_diagnostics) = parsed.into_parts();
//! assert!(syntax_diagnostics.is_empty());
//!
//! let bound = compiler::bind(document);
//! let (film, binding_diagnostics) = bound.into_parts();
//! assert!(binding_diagnostics.is_empty());
//!
//! let resolved = compiler::resolve(film.expect("the source contains one film"));
//! let (film, resolution_diagnostics) = resolved.into_parts();
//! assert!(resolution_diagnostics.is_empty());
//!
//! let rate = FrameRate::new(30, 1).expect("30 fps is valid");
//! let solved = compiler::solve(
//!     film.expect("the source resolves"),
//!     &BTreeMap::new(),
//!     Timebase::new(rate),
//! ).expect("the source references no external assets");
//! assert!(solved.diagnostics().is_empty());
//! assert_eq!(solved.timeline().expect("the film solves").interval().end().get(), 30);
//! ```

mod bind;
mod linked_film;
mod parse;
mod resolve;
mod resolved_film;
mod solve;

pub use bind::{BindReport, bind};
pub use linked_film::{
    LinkedCue, LinkedCues, LinkedElement, LinkedFilm, LinkedNode, LinkedOverlay, LinkedScene,
    LinkedShot, LinkedShotContent, LinkedVideo, LinkedVoiceOver,
};
pub use parse::{ParseReport, parse};
pub use resolve::{ResolveReport, resolve};
pub use resolved_film::{
    Authored, ResolvedCue, ResolvedCues, ResolvedElement, ResolvedFilm, ResolvedNode,
    ResolvedOverlay, ResolvedScene, ResolvedShot, ResolvedShotContent, ResolvedStart, ResolvedText,
    ResolvedVideo, ResolvedVoiceOver,
};
pub use solve::{SolveError, SolveReport, solve};
