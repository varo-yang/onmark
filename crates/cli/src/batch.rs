//! Bounded multi-variant rendering over one frozen screenplay and toolchain.
//!
//! The batch owns orchestration only. Every item still consumes the ordinary
//! compiler, Render Graph, Browser Plan, artifact cache, and local executor.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use onmark_core::compiler;
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::Timebase;
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_render::{EncodeProfile, RenderProfile};
use serde::{Deserialize, Serialize};

use crate::arguments::{BatchArgs, RenderArgs, source_directory};
use crate::assets::FrozenCatalog;
use crate::bundler::PresentationBundler;
use crate::compilation;
use crate::diagnostic::{self, AuthoredReport, JsonDiagnostic};
use crate::environment::Executables;
use crate::failure::CliError;
use crate::input::{self, BoundedReadError};
use crate::output;
use crate::progress::Progress;
use crate::render::{ExecutedRender, LocalRenderEngine, materialize_units};
use crate::variant::VariantImport;

const BATCH_MANIFEST_VERSION: u16 = 1;
const MAX_BATCH_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_BATCH_RENDERS: usize = 256;

pub(super) enum BatchOutcome {
    Rejected {
        report: AuthoredReport,
        json: bool,
    },
    Completed {
        report: AuthoredReport,
        renders: Vec<CompletedRender>,
        json: bool,
    },
}

impl BatchOutcome {
    pub(super) fn write(self) -> ExitCode {
        let result = match self {
            Self::Rejected { report, json } => write_rejected(&report, json),
            Self::Completed {
                report,
                renders,
                json,
            } => write_completed(&report, &renders, json),
        };
        result.unwrap_or(ExitCode::FAILURE)
    }

    fn rejected(path: PathBuf, source: String, diagnostics: Vec<Diagnostic>, json: bool) -> Self {
        Self::Rejected {
            report: AuthoredReport {
                path,
                source,
                diagnostics,
            },
            json,
        }
    }
}

pub(super) async fn run(args: BatchArgs, json: bool) -> Result<BatchOutcome, CliError> {
    let manifest = BatchManifest::load(&args.manifest)?;
    let jobs = manifest.into_jobs(&args);
    let first = jobs
        .first()
        .expect("a validated batch manifest contains at least one render");
    let screenplay = &first.screenplay;
    let source = read_screenplay(screenplay)?;
    let resolved = compilation::resolve(&source);
    let (film, diagnostics) = resolved.into_parts();
    let Some(film) = film else {
        return Ok(BatchOutcome::rejected(
            screenplay.clone(),
            source,
            diagnostics,
            json,
        ));
    };

    let mut films = Vec::with_capacity(jobs.len());
    for job in &jobs {
        match VariantImport::apply(job.variant.as_deref(), film.clone())? {
            VariantImport::Film(film) => films.push(film),
            VariantImport::Rejected(rejected) => {
                let (path, source, diagnostics) = rejected.into_parts();
                return Ok(BatchOutcome::rejected(path, source, diagnostics, json));
            }
        }
    }

    let encode_profile = common_encode_profile(&jobs)?;
    let profile =
        RenderProfile::new(first.width, first.height)?.with_alpha(encode_profile.alpha_mode());
    let executables = Executables::discover(first).await?;
    let probe = crate::render::ffprobe(executables.ffprobe.clone());
    let frozen = FrozenCatalog::freeze(&film, source_directory(screenplay), &probe).await?;
    let timelines = match solve_variants(
        films,
        frozen.facts(),
        Timebase::new(first.frame_rate),
        &diagnostics,
    )? {
        SolvedVariants::Ready(timelines) => timelines,
        SolvedVariants::Rejected(timing_diagnostics) => {
            return Ok(BatchOutcome::rejected(
                screenplay.clone(),
                source,
                timing_diagnostics,
                json,
            ));
        }
    };
    let partitions = common_partitions(&timelines)?;
    reject_existing_outputs(&jobs)?;

    let bundler = PresentationBundler::new(executables.bundler.clone());
    let bundle = bundler
        .bundle(&source, source_directory(screenplay), &partitions)
        .await?;
    let materialized = frozen.into_materialized()?;
    let cache = tempfile::Builder::new()
        .prefix("onmark-batch-cache-")
        .tempdir()
        .map_err(BatchError::TemporaryCache)?;
    let engine = LocalRenderEngine::for_batch(first, &executables, encode_profile, cache.path());
    let progress = Progress::for_command(json);
    let mut completed = Vec::with_capacity(jobs.len());

    for (job, timeline) in jobs.iter().zip(&timelines) {
        let output = job.output();
        output::create_parent(&output)?;
        let units = materialize_units(
            timeline,
            profile,
            &partitions,
            &bundle,
            materialized.assets(),
        )?;
        let executed = engine
            .execute(&partitions, &units, &output, progress)
            .await?;
        completed.push(CompletedRender::new(&executed));
    }

    Ok(BatchOutcome::Completed {
        report: AuthoredReport {
            path: screenplay.clone(),
            source,
            diagnostics,
        },
        renders: completed,
        json,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchManifest {
    version: u16,
    screenplay: String,
    renders: Vec<BatchManifestRender>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchManifestRender {
    variant: Option<String>,
    output: String,
}

impl BatchManifest {
    fn load(path: &Path) -> Result<Self, BatchError> {
        let source = input::read_utf8(path, MAX_BATCH_MANIFEST_BYTES).map_err(|source| {
            BatchError::Read {
                path: path.to_owned(),
                source,
            }
        })?;
        let manifest =
            serde_json::from_str::<Self>(&source).map_err(|source| BatchError::Parse {
                path: path.to_owned(),
                source,
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), BatchError> {
        if self.version != BATCH_MANIFEST_VERSION {
            return Err(BatchError::UnsupportedVersion(self.version));
        }
        if self.renders.is_empty() || self.renders.len() > MAX_BATCH_RENDERS {
            return Err(BatchError::InvalidRenderCount(self.renders.len()));
        }
        require_relative_path(&self.screenplay)?;

        let mut outputs = BTreeSet::new();
        for render in &self.renders {
            require_relative_path(&render.output)?;
            if let Some(variant) = &render.variant {
                require_relative_path(variant)?;
            }
            if !outputs.insert(render.output.clone()) {
                return Err(BatchError::DuplicateOutput(render.output.clone()));
            }
        }
        Ok(())
    }

    fn into_jobs(self, args: &BatchArgs) -> Vec<RenderArgs> {
        let root = args.manifest.parent().unwrap_or_else(|| Path::new("."));
        let screenplay = root.join(&self.screenplay);
        self.renders
            .into_iter()
            .map(|render| {
                args.render_args(
                    screenplay.clone(),
                    render.variant.map(|path| root.join(path)),
                    root.join(render.output),
                )
            })
            .collect()
    }
}

fn require_relative_path(path: &str) -> Result<(), BatchError> {
    let valid = !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."));
    if valid {
        Ok(())
    } else {
        Err(BatchError::InvalidPath(path.to_owned()))
    }
}

fn common_encode_profile(jobs: &[RenderArgs]) -> Result<EncodeProfile, CliError> {
    let Some((first, remaining)) = jobs.split_first() else {
        return Err(BatchError::InvalidRenderCount(0).into());
    };
    let expected = first.encode_profile()?;
    for job in remaining {
        if job.encode_profile()? != expected {
            return Err(BatchError::MixedOutputProfiles.into());
        }
    }
    Ok(expected)
}

fn solve_variants(
    films: Vec<compiler::ResolvedFilm>,
    assets: &std::collections::BTreeMap<
        onmark_core::model::AssetRef,
        onmark_core::model::FrozenAsset,
    >,
    timebase: Timebase,
    diagnostics: &[Diagnostic],
) -> Result<SolvedVariants, CliError> {
    let mut timelines = Vec::with_capacity(films.len());
    for film in films {
        let solved = compilation::solve(film, assets, timebase, diagnostics.to_vec())?;
        let (timeline, timing_diagnostics) = solved.into_parts();
        let Some(timeline) = timeline else {
            return Ok(SolvedVariants::Rejected(timing_diagnostics));
        };
        timelines.push(timeline);
    }
    Ok(SolvedVariants::Ready(timelines))
}

enum SolvedVariants {
    Ready(Vec<TimelineIr>),
    Rejected(Vec<Diagnostic>),
}

fn common_partitions(timelines: &[TimelineIr]) -> Result<PartitionPlan, BatchError> {
    let Some((first, remaining)) = timelines.split_first() else {
        return Err(BatchError::InvalidRenderCount(0));
    };
    let expected = RenderGraph::from_timeline(first, PresentationBundler::temporal_capability())
        .map_err(BatchError::RenderGraph)?
        .into_partition();
    for timeline in remaining {
        let actual =
            RenderGraph::from_timeline(timeline, PresentationBundler::temporal_capability())
                .map_err(BatchError::RenderGraph)?
                .into_partition();
        if actual != expected {
            return Err(BatchError::RenderDependencyChanged);
        }
    }
    Ok(expected)
}

fn reject_existing_outputs(jobs: &[RenderArgs]) -> Result<(), CliError> {
    for job in jobs {
        output::reject_existing(&job.output())?;
    }
    Ok(())
}

fn read_screenplay(path: &Path) -> Result<String, CliError> {
    let limit = u64::try_from(onmark_core::syntax::MAX_SCREENPLAY_BYTES)
        .expect("the screenplay byte limit fits in u64");
    input::read_utf8(path, limit).map_err(|source| CliError::read_screenplay(path, source))
}

pub(super) struct CompletedRender {
    output: PathBuf,
    frames: u64,
    reused_frames: u64,
    reused_regions: usize,
    regions: usize,
}

impl CompletedRender {
    fn new(executed: &ExecutedRender) -> Self {
        let reuse = executed.reuse();
        Self {
            output: executed.video().path().to_owned(),
            frames: executed.video().frames(),
            reused_frames: reuse.reused_frames(),
            reused_regions: reuse.reused_regions(),
            regions: reuse.regions(),
        }
    }
}

fn write_rejected(report: &AuthoredReport, json: bool) -> io::Result<ExitCode> {
    if json {
        write_json(report, None)?;
    } else {
        diagnostic::write_all(
            &mut io::stderr().lock(),
            &report.path,
            &report.source,
            &report.diagnostics,
        )?;
    }
    Ok(ExitCode::FAILURE)
}

fn write_completed(
    report: &AuthoredReport,
    renders: &[CompletedRender],
    json: bool,
) -> io::Result<ExitCode> {
    if json {
        write_json(report, Some(renders))?;
        return Ok(ExitCode::SUCCESS);
    }

    diagnostic::write_all(
        &mut io::stderr().lock(),
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Rendered {} variants", renders.len())?;
    for render in renders {
        writeln!(
            stdout,
            "{}: {} frames, reused {}/{} regions and {}/{} frames",
            render.output.display(),
            render.frames,
            render.reused_regions,
            render.regions,
            render.reused_frames,
            render.frames,
        )?;
    }
    Ok(ExitCode::SUCCESS)
}

fn write_json(report: &AuthoredReport, renders: Option<&[CompletedRender]>) -> io::Result<()> {
    let document = JsonBatchReport {
        version: 1,
        command: "batch",
        rendered: renders.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        renders: renders.map(|renders| renders.iter().map(JsonRender::from).collect()),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonBatchReport<'a> {
    version: u16,
    command: &'static str,
    rendered: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renders: Option<Vec<JsonRender>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonRender {
    output: String,
    frames: u64,
    reused_frames: u64,
    reused_regions: usize,
    regions: usize,
}

impl From<&CompletedRender> for JsonRender {
    fn from(render: &CompletedRender) -> Self {
        Self {
            output: render.output.display().to_string(),
            frames: render.frames,
            reused_frames: render.reused_frames,
            reused_regions: render.reused_regions,
            regions: render.regions,
        }
    }
}

#[derive(Debug)]
pub(super) enum BatchError {
    Read {
        path: PathBuf,
        source: BoundedReadError,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedVersion(u16),
    InvalidRenderCount(usize),
    InvalidPath(String),
    DuplicateOutput(String),
    MixedOutputProfiles,
    RenderDependencyChanged,
    RenderGraph(onmark_core::render_graph::InvalidRenderGraph),
    TemporaryCache(io::Error),
}

impl fmt::Display for BatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, .. } => {
                write!(
                    formatter,
                    "failed to read batch manifest {}",
                    path.display()
                )
            }
            Self::Parse { path, .. } => {
                write!(
                    formatter,
                    "failed to parse batch manifest {}",
                    path.display()
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "batch manifest version {version} is not supported"
                )
            }
            Self::InvalidRenderCount(count) => {
                write!(
                    formatter,
                    "batch manifest contains {count} renders; expected 1 to {MAX_BATCH_RENDERS}"
                )
            }
            Self::InvalidPath(path) => {
                write!(
                    formatter,
                    "batch path {path:?} must be a normalized relative path using forward slashes"
                )
            }
            Self::DuplicateOutput(path) => {
                write!(
                    formatter,
                    "batch output path {path:?} appears more than once"
                )
            }
            Self::MixedOutputProfiles => {
                formatter.write_str("every batch output must use the same delivery profile")
            }
            Self::RenderDependencyChanged => {
                formatter.write_str("variant values changed render dependencies")
            }
            Self::RenderGraph(source) => source.fmt(formatter),
            Self::TemporaryCache(_) => {
                formatter.write_str("failed to create the private batch cache")
            }
        }
    }
}

impl Error for BatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::RenderGraph(source) => Some(source),
            Self::TemporaryCache(source) => Some(source),
            Self::UnsupportedVersion(_)
            | Self::InvalidRenderCount(_)
            | Self::InvalidPath(_)
            | Self::DuplicateOutput(_)
            | Self::MixedOutputProfiles
            | Self::RenderDependencyChanged => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BATCH_MANIFEST_VERSION, BatchError, BatchManifest, BatchManifestRender, MAX_BATCH_RENDERS,
        require_relative_path,
    };

    #[test]
    fn accepts_only_normalized_relative_manifest_paths() {
        for path in ["film.html", "variants/summer.json", "renders/summer.mp4"] {
            require_relative_path(path).expect("the fixture path is normalized and relative");
        }

        for path in [
            "",
            "/film.html",
            "../film.html",
            "./film.html",
            "variants//summer.json",
            "variants/./summer.json",
            r"variants\summer.json",
            "C:/film.html",
            "renders/bad\0name.mp4",
        ] {
            assert!(matches!(
                require_relative_path(path),
                Err(BatchError::InvalidPath(_))
            ));
        }
    }

    #[test]
    fn rejects_unsupported_versions_duplicate_outputs_and_unbounded_work() {
        let mut manifest = manifest_with_renders(1);
        manifest.version = BATCH_MANIFEST_VERSION + 1;
        assert!(matches!(
            manifest.validate(),
            Err(BatchError::UnsupportedVersion(_))
        ));

        let mut manifest = manifest_with_renders(2);
        manifest.renders[1].output = manifest.renders[0].output.clone();
        assert!(matches!(
            manifest.validate(),
            Err(BatchError::DuplicateOutput(_))
        ));

        assert!(matches!(
            manifest_with_renders(0).validate(),
            Err(BatchError::InvalidRenderCount(0))
        ));
        assert!(matches!(
            manifest_with_renders(MAX_BATCH_RENDERS + 1).validate(),
            Err(BatchError::InvalidRenderCount(_))
        ));
    }

    fn manifest_with_renders(count: usize) -> BatchManifest {
        let renders = (0..count)
            .map(|index| BatchManifestRender {
                variant: Some(format!("variants/{index}.json")),
                output: format!("renders/{index}.mp4"),
            })
            .collect();
        BatchManifest {
            version: BATCH_MANIFEST_VERSION,
            screenplay: "film.html".into(),
            renders,
        }
    }
}
