//! Deterministic semantic checkpoints over solved production regions.
//!
//! The policy observes Timeline IR and Partition Plan facts only. It never
//! inspects presentation source or pixels to guess which frames matter.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use onmark_core::model::{FrameIndex, FrameInterval, SourceSpan};
use onmark_core::render_graph::PartitionPlan;
use onmark_core::timeline::{TimelineContent, TimelineElement, TimelineIr, TimelineTiming};

pub(super) const MAX_REVIEW_CHECKPOINTS: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewPlan {
    checkpoints: Vec<ReviewCheckpoint>,
}

impl ReviewPlan {
    pub(super) fn from_timeline(
        timeline: &TimelineIr,
        partitions: &PartitionPlan,
    ) -> Result<Self, ReviewPlanError> {
        let mut drafts = BTreeMap::new();
        add_regions(partitions, &mut drafts);
        add_timeline(timeline, &mut drafts);
        let checkpoints = finish_checkpoints(partitions, drafts)?;

        Ok(Self { checkpoints })
    }

    pub(super) fn checkpoints(&self) -> &[ReviewCheckpoint] {
        &self.checkpoints
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewCheckpoint {
    frame: FrameIndex,
    region: usize,
    position: u64,
    anchors: Vec<ReviewAnchor>,
}

impl ReviewCheckpoint {
    pub(super) const fn frame(&self) -> FrameIndex {
        self.frame
    }

    pub(super) const fn region(&self) -> usize {
        self.region
    }

    pub(super) const fn position(&self) -> u64 {
        self.position
    }

    pub(super) fn anchors(&self) -> &[ReviewAnchor] {
        &self.anchors
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewAnchor {
    reason: &'static str,
    subject: Option<ReviewSubject>,
}

impl ReviewAnchor {
    pub(super) const fn reason(&self) -> &'static str {
        self.reason
    }

    pub(super) const fn subject(&self) -> Option<&ReviewSubject> {
        self.subject.as_ref()
    }

    const fn region(reason: &'static str) -> Self {
        Self {
            reason,
            subject: None,
        }
    }

    fn for_subject(reason: &'static str, subject: ReviewSubject) -> Self {
        Self {
            reason,
            subject: Some(subject),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewSubject {
    kind: &'static str,
    id: Option<Box<str>>,
    spans: Vec<SourceSpan>,
    timing: TimelineFacts,
}

impl ReviewSubject {
    pub(super) const fn kind(&self) -> &'static str {
        self.kind
    }

    pub(super) fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub(super) fn spans(&self) -> &[SourceSpan] {
        &self.spans
    }

    pub(super) const fn timing(&self) -> &TimelineFacts {
        &self.timing
    }

    fn element(element: &TimelineElement, timing: &TimelineTiming) -> Self {
        Self {
            kind: element.kind().as_str(),
            id: element.id().map(|id| Box::from(id.as_str())),
            spans: vec![element.span()],
            timing: TimelineFacts::from_timing(timing),
        }
    }

    fn caption(interval: FrameInterval, timing_span: SourceSpan, text_span: SourceSpan) -> Self {
        Self {
            kind: "caption",
            id: None,
            spans: vec![timing_span, text_span],
            timing: TimelineFacts::new(interval, "authoredCaption", "authoredCaption", None, None),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TimelineFacts {
    interval: FrameInterval,
    start_reason: &'static str,
    end_reason: &'static str,
    start_authored_at: Option<SourceSpan>,
    end_authored_at: Option<SourceSpan>,
}

impl TimelineFacts {
    pub(super) const fn interval(&self) -> FrameInterval {
        self.interval
    }

    pub(super) const fn start_reason(&self) -> &'static str {
        self.start_reason
    }

    pub(super) const fn end_reason(&self) -> &'static str {
        self.end_reason
    }

    pub(super) const fn start_authored_at(&self) -> Option<SourceSpan> {
        self.start_authored_at
    }

    pub(super) const fn end_authored_at(&self) -> Option<SourceSpan> {
        self.end_authored_at
    }

    const fn new(
        interval: FrameInterval,
        start_reason: &'static str,
        end_reason: &'static str,
        start_authored_at: Option<SourceSpan>,
        end_authored_at: Option<SourceSpan>,
    ) -> Self {
        Self {
            interval,
            start_reason,
            end_reason,
            start_authored_at,
            end_authored_at,
        }
    }

    fn from_timing(timing: &TimelineTiming) -> Self {
        Self::new(
            timing.interval(),
            timing.start_reason().as_str(),
            timing.end_reason().as_str(),
            timing.start_reason().authored_at(),
            timing.end_reason().authored_at(),
        )
    }
}

#[derive(Default)]
struct DraftCheckpoint {
    anchors: Vec<ReviewAnchor>,
}

fn add_regions(partitions: &PartitionPlan, drafts: &mut BTreeMap<FrameIndex, DraftCheckpoint>) {
    for partition in partitions.units() {
        add_interval(
            drafts,
            partition.output(),
            [
                ReviewAnchor::region("regionStart"),
                ReviewAnchor::region("regionMiddle"),
                ReviewAnchor::region("regionEnd"),
            ],
        );
    }
}

fn add_timeline(timeline: &TimelineIr, drafts: &mut BTreeMap<FrameIndex, DraftCheckpoint>) {
    for shot in timeline.shots() {
        add_subject(
            drafts,
            shot.element(),
            shot.timing(),
            ["shotStart", "shotMiddle", "shotEnd"],
        );
        if let Some(transition) = shot.incoming_transition() {
            add_subject(
                drafts,
                transition.element(),
                transition.timing(),
                ["transitionStart", "transitionMiddle", "transitionEnd"],
            );
        }
        for content in shot.content() {
            match content {
                TimelineContent::Video(video) => add_edges(
                    drafts,
                    video.timing().interval(),
                    &ReviewSubject::element(video.element(), video.timing()),
                    ["videoStart", "videoEnd"],
                ),
                TimelineContent::Overlay(overlay) => add_edges(
                    drafts,
                    overlay.timing().interval(),
                    &ReviewSubject::element(overlay.element(), overlay.timing()),
                    ["overlayStart", "overlayEnd"],
                ),
                TimelineContent::VoiceOver(_) => {}
            }
        }
    }

    for caption in timeline.captions() {
        add_edges(
            drafts,
            caption.interval(),
            &ReviewSubject::caption(
                caption.interval(),
                caption.timing_span(),
                caption.text_span(),
            ),
            ["captionStart", "captionEnd"],
        );
    }
}

fn add_subject(
    drafts: &mut BTreeMap<FrameIndex, DraftCheckpoint>,
    element: &TimelineElement,
    timing: &TimelineTiming,
    reasons: [&'static str; 3],
) {
    let subject = ReviewSubject::element(element, timing);
    add_interval(
        drafts,
        timing.interval(),
        reasons.map(|reason| ReviewAnchor::for_subject(reason, subject.clone())),
    );
}

fn add_interval(
    drafts: &mut BTreeMap<FrameIndex, DraftCheckpoint>,
    interval: FrameInterval,
    anchors: [ReviewAnchor; 3],
) {
    let Some([start, middle, end]) = interval_points(interval) else {
        return;
    };
    for (frame, anchor) in [start, middle, end].into_iter().zip(anchors) {
        drafts.entry(frame).or_default().anchors.push(anchor);
    }
}

fn add_edges(
    drafts: &mut BTreeMap<FrameIndex, DraftCheckpoint>,
    interval: FrameInterval,
    subject: &ReviewSubject,
    reasons: [&'static str; 2],
) {
    let Some([start, _, end]) = interval_points(interval) else {
        return;
    };
    for (frame, reason) in [start, end].into_iter().zip(reasons) {
        drafts
            .entry(frame)
            .or_default()
            .anchors
            .push(ReviewAnchor::for_subject(reason, subject.clone()));
    }
}

fn interval_points(interval: FrameInterval) -> Option<[FrameIndex; 3]> {
    if interval.is_empty() {
        return None;
    }

    let start = interval.start().get();
    let end = interval.end().get() - 1;
    let middle = start + (end - start) / 2;
    Some([
        FrameIndex::new(start),
        FrameIndex::new(middle),
        FrameIndex::new(end),
    ])
}

fn finish_checkpoints(
    partitions: &PartitionPlan,
    drafts: BTreeMap<FrameIndex, DraftCheckpoint>,
) -> Result<Vec<ReviewCheckpoint>, ReviewPlanError> {
    if drafts.len() > MAX_REVIEW_CHECKPOINTS {
        return Err(ReviewPlanError::TooManyCheckpoints {
            actual: drafts.len(),
            maximum: MAX_REVIEW_CHECKPOINTS,
        });
    }

    drafts
        .into_iter()
        .map(|(frame, draft)| finish_checkpoint(partitions, frame, draft))
        .collect()
}

fn finish_checkpoint(
    partitions: &PartitionPlan,
    frame: FrameIndex,
    draft: DraftCheckpoint,
) -> Result<ReviewCheckpoint, ReviewPlanError> {
    let owner = partitions
        .units()
        .iter()
        .enumerate()
        .find(|(_, partition)| contains(partition.output(), frame));
    let Some((region, partition)) = owner else {
        return Err(ReviewPlanError::UnownedCheckpoint(frame));
    };
    let position = frame
        .get()
        .checked_sub(partition.output().start().get())
        .expect("the owning output starts no later than its checkpoint");

    Ok(ReviewCheckpoint {
        frame,
        region,
        position,
        anchors: draft.anchors,
    })
}

const fn contains(interval: FrameInterval, frame: FrameIndex) -> bool {
    interval.start().get() <= frame.get() && frame.get() < interval.end().get()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReviewPlanError {
    TooManyCheckpoints { actual: usize, maximum: usize },
    UnownedCheckpoint(FrameIndex),
}

impl fmt::Display for ReviewPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyCheckpoints { actual, maximum } => write!(
                formatter,
                "exact review requires {actual} checkpoints, exceeding the limit of {maximum}",
            ),
            Self::UnownedCheckpoint(frame) => write!(
                formatter,
                "exact review checkpoint {} has no publishing render region",
                frame.get(),
            ),
        }
    }
}

impl Error for ReviewPlanError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onmark_core::model::{FrameIndex, PresentationTemporalCapability, Timebase};
    use onmark_core::render_graph::RenderGraph;

    use super::{ReviewPlan, ReviewPlanError};
    use crate::compilation;

    #[test]
    fn merges_semantic_boundaries_into_region_owned_checkpoints() {
        let source = concat!(
            "<om-film><om-scene>",
            r#"<om-shot id="first" duration="1s"><om-title>First</om-title></om-shot>"#,
            r#"<om-transition duration="500ms"></om-transition>"#,
            r#"<om-shot id="second" duration="1s"><om-title>Second</om-title></om-shot>"#,
            "</om-scene></om-film>",
        );
        let resolved = compilation::resolve(source);
        let (film, diagnostics) = resolved.into_parts();
        assert!(diagnostics.is_empty());
        let solved = compilation::solve(
            film.expect("the fixture resolves"),
            &BTreeMap::new(),
            Timebase::new(
                onmark_core::model::FrameRate::new(30, 1).expect("the fixture rate is valid"),
            ),
            diagnostics,
        )
        .expect("the fixture solves");
        let (timeline, diagnostics) = solved.into_parts();
        assert!(diagnostics.is_empty());
        let timeline = timeline.expect("the fixture has Timeline IR");
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the fixture graph is complete")
                .into_partition();

        let review =
            ReviewPlan::from_timeline(&timeline, &partitions).expect("the review plan is bounded");
        let transition_start = review
            .checkpoints()
            .iter()
            .find(|checkpoint| checkpoint.frame() == FrameIndex::new(15))
            .expect("the overlap start is a checkpoint");

        assert_eq!(transition_start.region(), 1);
        assert_eq!(transition_start.position(), 0);
        assert!(
            transition_start
                .anchors()
                .iter()
                .any(|anchor| anchor.reason() == "transitionStart"),
        );
        assert!(
            transition_start
                .anchors()
                .iter()
                .filter_map(|anchor| anchor.subject())
                .any(|subject| subject.kind() == "om-transition"),
        );
    }

    #[test]
    fn rejects_a_review_that_cannot_retain_every_semantic_checkpoint() {
        use std::fmt::Write as _;

        let mut source = String::from("<om-film><om-scene>");
        for index in 0..180 {
            write!(source, r#"<om-shot id="s{index}" duration="1s"></om-shot>"#)
                .expect("writing into a String cannot fail");
        }
        source.push_str("</om-scene></om-film>");
        let resolved = compilation::resolve(&source);
        let (film, diagnostics) = resolved.into_parts();
        assert!(diagnostics.is_empty());
        let solved = compilation::solve(
            film.expect("the fixture resolves"),
            &BTreeMap::new(),
            Timebase::new(
                onmark_core::model::FrameRate::new(30, 1).expect("the fixture rate is valid"),
            ),
            diagnostics,
        )
        .expect("the fixture solves");
        let (timeline, diagnostics) = solved.into_parts();
        assert!(diagnostics.is_empty());
        let timeline = timeline.expect("the fixture has Timeline IR");
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the fixture graph is complete")
                .into_partition();

        let error = ReviewPlan::from_timeline(&timeline, &partitions)
            .expect_err("the review must not silently sample a large film");

        assert!(matches!(error, ReviewPlanError::TooManyCheckpoints { .. },));
    }
}
