//! Versioned static review artifacts and bounded prior-manifest comparison.
//!
//! Persisted bytes contain deterministic compiler and pixel evidence only.
//! Timings and cache-hit state remain command observations.

use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use onmark_core::model::{FrameInterval, SourceSpan};
use onmark_core::render_graph::PartitionPlan;
use onmark_core::timeline::TimelineIr;
use onmark_render::{CapturedFrame, FrameArtifact, RenderProfile};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::plan::{ReviewAnchor, ReviewCheckpoint, ReviewPlan, ReviewSubject, TimelineFacts};

const MANIFEST_VERSION: u16 = 1;
const MANIFEST_FILE: &str = "manifest.json";
const CONTACT_SHEET_FILE: &str = "index.html";
const FRAME_DIRECTORY: &str = "frames";
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const REVIEW_ID_HEX_CHARS: usize = 12;

pub(super) struct ReviewDocument {
    manifest: ReviewManifest,
    manifest_bytes: Vec<u8>,
    contact_sheet: String,
    frames: Vec<CapturedFrame>,
    id: String,
}

impl ReviewDocument {
    pub(super) fn build(
        source: &str,
        timeline: &TimelineIr,
        profile: RenderProfile,
        partitions: &PartitionPlan,
        plan: &ReviewPlan,
        artifacts: &[FrameArtifact],
        frames: Vec<CapturedFrame>,
    ) -> Result<Self, ReviewReportError> {
        let manifest = ReviewManifest::build(
            source, timeline, profile, partitions, plan, artifacts, &frames,
        )?;
        let mut manifest_bytes =
            serde_json::to_vec_pretty(&manifest).map_err(ReviewReportError::EncodeManifest)?;
        manifest_bytes.push(b'\n');
        let id = digest_prefix(&manifest_bytes, REVIEW_ID_HEX_CHARS);
        let contact_sheet = contact_sheet(&manifest);

        Ok(Self {
            manifest,
            manifest_bytes,
            contact_sheet,
            frames,
            id,
        })
    }

    pub(super) fn default_output(&self, screenplay: &Path) -> PathBuf {
        let stem = screenplay
            .file_stem()
            .unwrap_or(screenplay.as_os_str())
            .to_string_lossy();
        Path::new("reviews").join(format!("{stem}-{}", self.id))
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn regions(&self) -> usize {
        self.manifest.regions.len()
    }

    pub(super) fn checkpoints(&self) -> usize {
        self.manifest.checkpoints.len()
    }

    pub(super) fn compare(&self, prior: &ReviewBaseline) -> ReviewComparison {
        ReviewComparison::between(&prior.manifest, &self.manifest)
    }

    pub(super) fn publish(
        &self,
        output: &Path,
        allow_exact_reuse: bool,
    ) -> Result<ReviewPublication, ReviewReportError> {
        if output.exists() {
            if allow_exact_reuse && self.matches_existing(output)? {
                return Ok(ReviewPublication::Reused);
            }
            return Err(ReviewReportError::OutputExists(output.to_owned()));
        }

        let parent = output.parent().filter(|path| !path.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ReviewReportError::CreateParent {
            path: parent.to_owned(),
            source,
        })?;
        let staging = tempfile::Builder::new()
            .prefix(".onmark-review-")
            .tempdir_in(parent)
            .map_err(ReviewReportError::Stage)?;
        self.write_staging(staging.path())?;
        fs::rename(staging.keep(), output).map_err(|source| ReviewReportError::Publish {
            path: output.to_owned(),
            source,
        })?;

        Ok(ReviewPublication::Published)
    }

    fn write_staging(&self, staging: &Path) -> Result<(), ReviewReportError> {
        let frames_directory = staging.join(FRAME_DIRECTORY);
        fs::create_dir(&frames_directory).map_err(|source| ReviewReportError::Write {
            path: frames_directory.clone(),
            source,
        })?;
        for (checkpoint, frame) in self.manifest.checkpoints.iter().zip(&self.frames) {
            let path = staging.join(&checkpoint.png);
            fs::write(&path, frame.png().as_bytes())
                .map_err(|source| ReviewReportError::Write { path, source })?;
        }
        write_file(staging.join(MANIFEST_FILE), &self.manifest_bytes)?;
        write_file(
            staging.join(CONTACT_SHEET_FILE),
            self.contact_sheet.as_bytes(),
        )
    }

    fn matches_existing(&self, output: &Path) -> Result<bool, ReviewReportError> {
        let metadata = fs::symlink_metadata(output).map_err(|source| ReviewReportError::Read {
            path: output.to_owned(),
            source,
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Ok(false);
        }
        let manifest = output.join(MANIFEST_FILE);
        let existing = read_bounded(&manifest)?;
        if existing != self.manifest_bytes {
            return Ok(false);
        }
        let contact_sheet = output.join(CONTACT_SHEET_FILE);
        let existing = read_bounded(&contact_sheet)?;
        if existing != self.contact_sheet.as_bytes() {
            return Ok(false);
        }
        for checkpoint in &self.manifest.checkpoints {
            let png = output.join(&checkpoint.png);
            let bytes = read_bounded_with_limit(&png, checkpoint.png_bytes)?;
            if digest_hex(&bytes) != checkpoint.png_sha256 {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) struct ReviewBaseline {
    path: PathBuf,
    manifest: ReviewManifest,
}

impl ReviewBaseline {
    pub(super) fn load(path: PathBuf) -> Result<Self, ReviewReportError> {
        let manifest = read_manifest(&path)?;
        Ok(Self { path, manifest })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReviewPublication {
    Published,
    Reused,
}

impl fmt::Display for ReviewPublication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Published => formatter.write_str("published"),
            Self::Reused => formatter.write_str("reused"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewComparison {
    #[serde(rename = "unchangedRegions")]
    unchanged: usize,
    #[serde(rename = "changedRegions")]
    changed: usize,
    #[serde(rename = "addedRegions")]
    added: usize,
    #[serde(rename = "removedRegions")]
    removed: usize,
}

impl ReviewComparison {
    fn between(previous: &ReviewManifest, current: &ReviewManifest) -> Self {
        let paired = previous.regions.len().min(current.regions.len());
        let mut unchanged_regions = 0;
        let mut changed_regions = 0;
        for index in 0..paired {
            if previous.regions[index].artifact_id == current.regions[index].artifact_id {
                unchanged_regions += 1;
            } else {
                changed_regions += 1;
            }
        }

        Self {
            unchanged: unchanged_regions,
            changed: changed_regions,
            added: current.regions.len().saturating_sub(paired),
            removed: previous.regions.len().saturating_sub(paired),
        }
    }

    pub(super) const fn unchanged_regions(self) -> usize {
        self.unchanged
    }

    pub(super) const fn changed_regions(self) -> usize {
        self.changed
    }

    pub(super) const fn added_regions(self) -> usize {
        self.added
    }

    pub(super) const fn removed_regions(self) -> usize {
        self.removed
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReviewManifest {
    version: u16,
    source_sha256: String,
    timeline_version: u16,
    frame_rate: FrameRateFact,
    profile: RenderProfile,
    regions: Vec<RegionFact>,
    checkpoints: Vec<CheckpointFact>,
}

impl ReviewManifest {
    fn build(
        source: &str,
        timeline: &TimelineIr,
        profile: RenderProfile,
        partitions: &PartitionPlan,
        plan: &ReviewPlan,
        artifacts: &[FrameArtifact],
        frames: &[CapturedFrame],
    ) -> Result<Self, ReviewReportError> {
        ensure_lengths(
            partitions.units().len(),
            artifacts.len(),
            "render region and frame artifact counts differ",
        )?;
        ensure_lengths(
            plan.checkpoints().len(),
            frames.len(),
            "review checkpoint and captured frame counts differ",
        )?;
        let regions = partitions
            .units()
            .iter()
            .zip(artifacts)
            .enumerate()
            .map(|(index, (partition, artifact))| RegionFact {
                index,
                evaluation: partition.evaluation().into(),
                output: partition.output().into(),
                shots: partition.shots().map(|shot| shot.get()).collect(),
                artifact_id: artifact.id().to_string(),
            })
            .collect();
        let checkpoints = plan
            .checkpoints()
            .iter()
            .zip(frames)
            .map(|(checkpoint, frame)| CheckpointFact::new(checkpoint, frame))
            .collect();
        let rate = timeline.timebase().frame_rate();

        Ok(Self {
            version: MANIFEST_VERSION,
            source_sha256: digest_hex(source.as_bytes()),
            timeline_version: timeline.version().get(),
            frame_rate: FrameRateFact {
                numerator: rate.numerator(),
                denominator: rate.denominator(),
            },
            profile,
            regions,
            checkpoints,
        })
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FrameRateFact {
    numerator: u32,
    denominator: u32,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RegionFact {
    index: usize,
    evaluation: IntervalFact,
    output: IntervalFact,
    shots: Vec<usize>,
    artifact_id: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CheckpointFact {
    frame: u64,
    region: usize,
    artifact_position: u64,
    png: String,
    png_bytes: u64,
    png_sha256: String,
    raw_rgba_sha256: String,
    anchors: Vec<AnchorFact>,
}

impl CheckpointFact {
    fn new(checkpoint: &ReviewCheckpoint, frame: &CapturedFrame) -> Self {
        let png = format!(
            "{FRAME_DIRECTORY}/frame-{:012}.png",
            checkpoint.frame().get()
        );
        Self {
            frame: checkpoint.frame().get(),
            region: checkpoint.region(),
            artifact_position: checkpoint.position(),
            png,
            png_bytes: u64::try_from(frame.png().as_bytes().len())
                .expect("one bounded PNG length fits in u64"),
            png_sha256: digest_hex(frame.png().as_bytes()),
            raw_rgba_sha256: frame.raw_rgba_hash().to_string(),
            anchors: checkpoint.anchors().iter().map(AnchorFact::from).collect(),
        }
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AnchorFact {
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<SubjectFact>,
}

impl From<&ReviewAnchor> for AnchorFact {
    fn from(anchor: &ReviewAnchor) -> Self {
        Self {
            reason: anchor.reason().to_owned(),
            subject: anchor.subject().map(SubjectFact::from),
        }
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SubjectFact {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    spans: Vec<SpanFact>,
    timing: TimingFact,
}

impl From<&ReviewSubject> for SubjectFact {
    fn from(subject: &ReviewSubject) -> Self {
        Self {
            kind: subject.kind().to_owned(),
            id: subject.id().map(str::to_owned),
            spans: subject
                .spans()
                .iter()
                .copied()
                .map(SpanFact::from)
                .collect(),
            timing: subject.timing().into(),
        }
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TimingFact {
    interval: IntervalFact,
    start_reason: String,
    end_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_authored_at: Option<SpanFact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_authored_at: Option<SpanFact>,
}

impl From<&TimelineFacts> for TimingFact {
    fn from(timing: &TimelineFacts) -> Self {
        Self {
            interval: timing.interval().into(),
            start_reason: timing.start_reason().to_owned(),
            end_reason: timing.end_reason().to_owned(),
            start_authored_at: timing.start_authored_at().map(SpanFact::from),
            end_authored_at: timing.end_authored_at().map(SpanFact::from),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct IntervalFact {
    start: u64,
    end: u64,
}

impl From<FrameInterval> for IntervalFact {
    fn from(interval: FrameInterval) -> Self {
        Self {
            start: interval.start().get(),
            end: interval.end().get(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SpanFact {
    source: u32,
    start: u64,
    end: u64,
}

impl From<SourceSpan> for SpanFact {
    fn from(span: SourceSpan) -> Self {
        Self {
            source: span.source().get(),
            start: span.start().get(),
            end: span.end().get(),
        }
    }
}

fn contact_sheet(manifest: &ReviewManifest) -> String {
    let mut html = String::with_capacity(manifest.checkpoints.len().saturating_mul(512));
    write_contact_sheet(&mut html, manifest).expect("writing into a String cannot fail");
    html
}

fn write_contact_sheet(output: &mut String, manifest: &ReviewManifest) -> fmt::Result {
    write!(
        output,
        concat!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
            "<title>Onmark exact review</title><style>{style}</style></head>",
            "<body><header><p>ONMARK EXACT REVIEW</p>",
            "<h1>{checkpoints} checkpoints · {regions} regions</h1>",
            "<p class=\"meta\">Every image is a verified production frame. ",
            "See <a href=\"manifest.json\">manifest.json</a> for exact provenance.</p>",
            "</header><main>",
        ),
        style = CONTACT_SHEET_CSS,
        checkpoints = manifest.checkpoints.len(),
        regions = manifest.regions.len(),
    )?;
    for checkpoint in &manifest.checkpoints {
        write_checkpoint_card(output, checkpoint)?;
    }
    output.write_str("</main></body></html>")
}

fn write_checkpoint_card(output: &mut String, checkpoint: &CheckpointFact) -> fmt::Result {
    output.write_str("<article><img loading=\"lazy\" src=\"")?;
    escape_html(output, &checkpoint.png)?;
    write!(
        output,
        concat!(
            "\" alt=\"Exact frame {frame}\"><div>",
            "<strong>Frame {frame}</strong><span>Region {region}</span><p>",
        ),
        frame = checkpoint.frame,
        region = checkpoint.region,
    )?;
    write_reasons(output, &checkpoint.anchors)?;
    output.write_str("</p><code>")?;
    escape_html(output, &checkpoint.raw_rgba_sha256)?;
    output.write_str("</code></div></article>")
}

fn write_reasons(output: &mut String, anchors: &[AnchorFact]) -> fmt::Result {
    for (index, anchor) in anchors.iter().enumerate() {
        if index > 0 {
            output.write_str(" · ")?;
        }
        escape_html(output, &anchor.reason)?;
    }
    Ok(())
}

const CONTACT_SHEET_CSS: &str = concat!(
    ":root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;",
    "background:#0a0a0a;color:#f2f2f2}*{box-sizing:border-box}body{margin:0;padding:40px}",
    "header{max-width:960px;margin:0 auto 32px}header>p:first-child{letter-spacing:.14em;",
    "font-size:12px;color:#8f8f8f}h1{font-size:clamp(32px,6vw,72px);margin:.15em 0}",
    ".meta{color:#aaa}a{color:inherit}main{display:grid;grid-template-columns:",
    "repeat(auto-fit,minmax(320px,1fr));gap:18px;max-width:1800px;margin:auto}",
    "article{overflow:hidden;border:1px solid #292929;border-radius:14px;background:#111}",
    "img{display:block;width:100%;height:auto;background:#050505}article div{padding:14px}",
    "strong{font-size:18px}span{float:right;color:#999}article p{color:#bbb;margin:.6em 0}",
    "code{font-size:10px;color:#777;overflow-wrap:anywhere}@media(max-width:600px){",
    "body{padding:18px}main{grid-template-columns:1fr}}",
);

fn escape_html(output: &mut String, value: &str) -> fmt::Result {
    for character in value.chars() {
        match character {
            '&' => output.write_str("&amp;")?,
            '<' => output.write_str("&lt;")?,
            '>' => output.write_str("&gt;")?,
            '"' => output.write_str("&quot;")?,
            '\'' => output.write_str("&#39;")?,
            _ => output.write_char(character)?,
        }
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<ReviewManifest, ReviewReportError> {
    let bytes = read_bounded(path)?;
    let manifest: ReviewManifest =
        serde_json::from_slice(&bytes).map_err(|source| ReviewReportError::ParseManifest {
            path: path.to_owned(),
            source,
        })?;
    if manifest.version != MANIFEST_VERSION {
        return Err(ReviewReportError::UnsupportedManifest {
            path: path.to_owned(),
            version: manifest.version,
        });
    }
    Ok(manifest)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, ReviewReportError> {
    read_bounded_with_limit(path, MAX_MANIFEST_BYTES)
}

fn read_bounded_with_limit(path: &Path, maximum: u64) -> Result<Vec<u8>, ReviewReportError> {
    let path_metadata = fs::symlink_metadata(path).map_err(|source| ReviewReportError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !path_metadata.is_file()
        || path_metadata.file_type().is_symlink()
        || path_metadata.len() > maximum
    {
        return Err(ReviewReportError::InvalidFile {
            path: path.to_owned(),
            maximum,
        });
    }

    let file = File::open(path).map_err(|source| ReviewReportError::Read {
        path: path.to_owned(),
        source,
    })?;
    let file_metadata = file.metadata().map_err(|source| ReviewReportError::Read {
        path: path.to_owned(),
        source,
    })?;
    if !file_metadata.is_file() || file_metadata.len() > maximum {
        return Err(ReviewReportError::InvalidFile {
            path: path.to_owned(),
            maximum,
        });
    }

    // The handle may outlive a path replacement. Bound the read itself instead
    // of trusting either metadata observation as an allocation guarantee.
    let read_limit = maximum.saturating_add(1);
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| ReviewReportError::Read {
            path: path.to_owned(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        return Err(ReviewReportError::InvalidFile {
            path: path.to_owned(),
            maximum,
        });
    }
    Ok(bytes)
}

fn write_file(path: PathBuf, bytes: &[u8]) -> Result<(), ReviewReportError> {
    fs::write(&path, bytes).map_err(|source| ReviewReportError::Write { path, source })
}

fn ensure_lengths(
    expected: usize,
    actual: usize,
    message: &'static str,
) -> Result<(), ReviewReportError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ReviewReportError::InternalCount {
            message,
            expected,
            actual,
        })
    }
}

fn digest_prefix(bytes: &[u8], length: usize) -> String {
    digest_hex(bytes).chars().take(length).collect()
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing into a String cannot fail");
    }
    encoded
}

#[derive(Debug)]
pub(crate) enum ReviewReportError {
    InternalCount {
        message: &'static str,
        expected: usize,
        actual: usize,
    },
    EncodeManifest(serde_json::Error),
    ParseManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedManifest {
        path: PathBuf,
        version: u16,
    },
    InvalidFile {
        path: PathBuf,
        maximum: u64,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    CreateParent {
        path: PathBuf,
        source: io::Error,
    },
    Stage(io::Error),
    Write {
        path: PathBuf,
        source: io::Error,
    },
    Publish {
        path: PathBuf,
        source: io::Error,
    },
    OutputExists(PathBuf),
}

impl fmt::Display for ReviewReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InternalCount {
                message,
                expected,
                actual,
            } => write!(
                formatter,
                "{message}: expected {expected}, received {actual}"
            ),
            Self::EncodeManifest(_) => {
                formatter.write_str("failed to encode the exact review manifest")
            }
            Self::ParseManifest { path, .. } => {
                write!(
                    formatter,
                    "failed to parse review manifest {}",
                    path.display()
                )
            }
            Self::UnsupportedManifest { path, version } => write!(
                formatter,
                "review manifest {} uses unsupported version {version}",
                path.display(),
            ),
            Self::InvalidFile { path, maximum } => write!(
                formatter,
                "review file {} is not a regular file within the {maximum}-byte limit",
                path.display(),
            ),
            Self::Read { path, .. } => {
                write!(formatter, "failed to read review file {}", path.display())
            }
            Self::CreateParent { path, .. } => write!(
                formatter,
                "failed to create review output parent {}",
                path.display(),
            ),
            Self::Stage(_) => formatter.write_str("failed to create the private review staging"),
            Self::Write { path, .. } => {
                write!(formatter, "failed to write review file {}", path.display())
            }
            Self::Publish { path, .. } => {
                write!(
                    formatter,
                    "failed to publish review directory {}",
                    path.display()
                )
            }
            Self::OutputExists(path) => {
                write!(formatter, "review output {} already exists", path.display())
            }
        }
    }
}

impl Error for ReviewReportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EncodeManifest(source) | Self::ParseManifest { source, .. } => Some(source),
            Self::Read { source, .. }
            | Self::CreateParent { source, .. }
            | Self::Write { source, .. }
            | Self::Publish { source, .. }
            | Self::Stage(source) => Some(source),
            Self::InternalCount { .. }
            | Self::UnsupportedManifest { .. }
            | Self::InvalidFile { .. }
            | Self::OutputExists(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        ReviewComparison, ReviewManifest, ReviewReportError, escape_html, read_bounded_with_limit,
    };

    #[test]
    fn comparison_never_authorizes_reuse_across_changed_region_identity() {
        let previous: ReviewManifest =
            serde_json::from_str(&manifest_json(&["sha256:a", "sha256:b"]))
                .expect("the previous manifest fixture is valid");
        let current: ReviewManifest =
            serde_json::from_str(&manifest_json(&["sha256:a", "sha256:c", "sha256:d"]))
                .expect("the current manifest fixture is valid");

        let comparison = ReviewComparison::between(&previous, &current);

        assert_eq!(comparison.unchanged_regions(), 1);
        assert_eq!(comparison.changed_regions(), 1);
        assert_eq!(comparison.added_regions(), 1);
        assert_eq!(comparison.removed_regions(), 0);
    }

    #[test]
    fn bounded_read_rejects_a_file_larger_than_its_limit() {
        let directory = tempfile::tempdir().expect("the test directory can be created");
        let path = directory.path().join("oversized");
        fs::write(&path, b"12345").expect("the fixture can be written");

        let error =
            read_bounded_with_limit(&path, 4).expect_err("the reader must enforce its byte limit");

        assert!(matches!(error, ReviewReportError::InvalidFile { .. }));
    }

    #[test]
    fn contact_sheet_text_escapes_html_control_characters() {
        let mut escaped = String::new();

        escape_html(&mut escaped, r#"<frame id="one">&'</frame>"#)
            .expect("writing into a String cannot fail");

        assert_eq!(
            escaped,
            "&lt;frame id=&quot;one&quot;&gt;&amp;&#39;&lt;/frame&gt;"
        );
    }

    fn manifest_json(artifact_ids: &[&str]) -> String {
        let regions = artifact_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                format!(
                    r#"{{"index":{index},"evaluation":{{"start":0,"end":1}},"output":{{"start":0,"end":1}},"shots":[],"artifactId":"{id}"}}"#,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"version":1,"sourceSha256":"sha256","timelineVersion":1,"frameRate":{{"numerator":30,"denominator":1}},"profile":{{"width":2,"height":2,"alpha":"opaque"}},"regions":[{regions}],"checkpoints":[]}}"#,
        )
    }
}
