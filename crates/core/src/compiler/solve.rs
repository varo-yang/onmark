//! Exact timeline solving from typed intent and frozen asset facts.
//!
//! The solver alone converts duration into frames and assigns absolute
//! intervals. A failed duration invalidates downstream placement without
//! preventing independent diagnostics from being collected.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{
    AssetMetadata, AssetRef, AudioMetadata, CueId, Duration, EventRef, FrameCount, FrameIndex,
    FrameInterval, FrozenAsset, FrozenAssetId, Rounding, SourceSpan, Timebase, VideoMetadata,
};
use crate::timeline::{
    TimelineContent, TimelineElement, TimelineEvent, TimelineIr, TimelineOverlay, TimelineScene,
    TimelineShot, TimelineText, TimelineTiming, TimelineVideo, TimelineVoiceOver, TimingReason,
};

use super::resolved_film::{
    Authored, ResolvedCues, ResolvedElement, ResolvedFilm, ResolvedMedia, ResolvedOverlay,
    ResolvedScene, ResolvedShot, ResolvedShotContent, ResolvedStart, ResolvedText, ResolvedVideo,
    ResolvedVoiceOver,
};

/// Optional Timeline IR and the authored diagnostics produced while solving it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SolveReport {
    timeline: Option<TimelineIr>,
    diagnostics: Diagnostics,
}

impl SolveReport {
    /// Returns Timeline IR when no error diagnostic was produced.
    #[must_use]
    pub const fn timeline(&self) -> Option<&TimelineIr> {
        self.timeline.as_ref()
    }

    /// Returns all authored diagnostics produced during timing.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Returns the optional timeline and its authored diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<TimelineIr>, Diagnostics) {
        (self.timeline, self.diagnostics)
    }
}

/// Solves exact frame facts from a resolved film and frozen assets.
///
/// # Errors
///
/// Returns [`SolveError`] when a referenced asset has not been probed. Authored
/// timing mistakes remain accumulated diagnostics in [`SolveReport`].
pub fn solve(
    film: ResolvedFilm,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
    timebase: Timebase,
) -> Result<SolveReport, SolveError> {
    Solver::new(assets, timebase).solve(film)
}

/// Infrastructure input required by deterministic timeline solving is absent.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SolveError {
    /// A logical source has no frozen identity and normalized probe facts.
    MissingFrozenAsset(AssetRef),
}

impl fmt::Display for SolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrozenAsset(asset) => {
                write!(formatter, "frozen asset is missing for \"{asset}\"")
            }
        }
    }
}

impl Error for SolveError {}

/// Single mutable owner of event facts, sequential placement, and diagnostics.
struct Solver<'a> {
    assets: &'a BTreeMap<AssetRef, FrozenAsset>,
    timebase: Timebase,
    events: BTreeMap<CueId, TimelineEvent>,
    diagnostics: Diagnostics,
    cursor: PlacementCursor,
}

impl<'a> Solver<'a> {
    fn new(assets: &'a BTreeMap<AssetRef, FrozenAsset>, timebase: Timebase) -> Self {
        Self {
            assets,
            timebase,
            events: BTreeMap::new(),
            diagnostics: Diagnostics::new(),
            cursor: PlacementCursor::FilmStart,
        }
    }

    fn solve(mut self, film: ResolvedFilm) -> Result<SolveReport, SolveError> {
        let (element, cues, scenes, _ids) = film.into_parts();
        self.solve_events(cues);

        let mut timeline_scenes = Vec::with_capacity(scenes.len());
        for scene in scenes {
            timeline_scenes.push(self.solve_scene(scene)?);
        }

        let interval = interval(FrameIndex::ZERO, self.cursor.position());
        let candidate = TimelineIr::new(
            self.timebase,
            timeline_element(element),
            interval,
            self.events,
            timeline_scenes,
        );
        let timeline = (!self.diagnostics.has_errors()).then_some(candidate);

        Ok(SolveReport {
            timeline,
            diagnostics: self.diagnostics,
        })
    }

    fn solve_events(&mut self, cues: Option<ResolvedCues>) {
        let Some(cues) = cues else {
            return;
        };
        let (_element, cues) = cues.into_parts();

        for cue in cues {
            let (id, time, _span) = cue.into_parts();
            let (id, _id_span) = id.into_parts();
            let (time, authored_at) = time.into_parts();
            let Some(at) = frame_at(self.timebase, time, authored_at, &mut self.diagnostics) else {
                continue;
            };

            self.events.insert(id, TimelineEvent::new(at, authored_at));
        }
    }

    fn solve_scene(&mut self, scene: ResolvedScene) -> Result<TimelineScene, SolveError> {
        let (element, shots) = scene.into_parts();
        let (start, start_reason) = self.cursor.scene_start();
        let mut timeline_shots = Vec::with_capacity(shots.len());

        for shot in shots {
            if let Some(shot) = self.solve_shot(shot)? {
                timeline_shots.push(shot);
            }
        }

        let timing = TimelineTiming::new(
            interval(start, self.cursor.position()),
            start_reason,
            TimingReason::Children,
        );
        let element = timeline_element(element);

        Ok(TimelineScene::new(element, timing, timeline_shots))
    }

    fn solve_shot(&mut self, shot: ResolvedShot) -> Result<Option<TimelineShot>, SolveError> {
        let (element, duration, content) = shot.into_parts();
        let source = element.span();
        let prepared = self.prepare_contents(content)?;
        let explicit = self.explicit_duration(duration);
        let duration = self.shot_duration(explicit, prepared.primary, source);
        let Some(timing) = self.place_shot(duration, source) else {
            return Ok(None);
        };
        let content = self.lower_contents(prepared.content, timing.interval());
        let element = timeline_element(element);
        let shot = TimelineShot::new(element, timing, content);

        Ok(Some(shot))
    }

    fn prepare_contents(
        &mut self,
        content: Vec<ResolvedShotContent>,
    ) -> Result<PreparedShot, SolveError> {
        let mut prepared = Vec::with_capacity(content.len());
        let mut primary = PrimaryDuration::Absent;

        for content in content {
            let is_primary = is_primary_content(&content);
            if let Some(content) = self.prepare_content(content)? {
                if let Some(end) = content.primary_end() {
                    primary.include(end);
                }
                prepared.push(content);
            } else if is_primary {
                primary.reject();
            }
        }

        Ok(PreparedShot {
            content: prepared,
            primary,
        })
    }

    fn lower_contents(
        &mut self,
        content: Vec<PreparedContent>,
        shot: FrameInterval,
    ) -> Vec<TimelineContent> {
        let mut timeline = Vec::with_capacity(content.len());
        for content in content {
            if let Some(content) = self.lower_content(content, shot) {
                timeline.push(content);
            }
        }

        timeline
    }

    fn place_shot(
        &mut self,
        duration: Option<ShotDuration>,
        source: SourceSpan,
    ) -> Option<TimelineTiming> {
        let (start, start_reason) = self.cursor.next_shot()?;

        let Some(duration) = duration else {
            self.cursor = PlacementCursor::Lost(start);
            return None;
        };
        let Some(end) = advance(start, duration.frames, source, &mut self.diagnostics) else {
            self.cursor = PlacementCursor::Lost(start);
            return None;
        };

        self.cursor = PlacementCursor::Sequential(end);
        Some(TimelineTiming::new(
            interval(start, end),
            start_reason,
            duration.reason,
        ))
    }

    fn prepare_content(
        &mut self,
        content: ResolvedShotContent,
    ) -> Result<Option<PreparedContent>, SolveError> {
        match content {
            ResolvedShotContent::Video(video) => self.prepare_video(video),
            ResolvedShotContent::VoiceOver(voice_over) => self.prepare_voice_over(voice_over),
            ResolvedShotContent::Overlay(overlay) => Ok(Some(PreparedContent::Overlay(overlay))),
        }
    }

    fn prepare_video(
        &mut self,
        video: ResolvedVideo,
    ) -> Result<Option<PreparedContent>, SolveError> {
        let media = self.prepare_media(video.into_media(), MediaTrack::Video)?;
        Ok(media.map(PreparedContent::Video))
    }

    fn prepare_voice_over(
        &mut self,
        voice_over: ResolvedVoiceOver,
    ) -> Result<Option<PreparedContent>, SolveError> {
        let (media, text) = voice_over.into_parts();
        let Some(media) = self.prepare_media(media, MediaTrack::Audio)? else {
            return Ok(None);
        };

        Ok(Some(PreparedContent::VoiceOver { media, text }))
    }

    fn prepare_media(
        &mut self,
        media: ResolvedMedia,
        track: MediaTrack,
    ) -> Result<Option<PreparedMedia>, SolveError> {
        let (element, source, delay) = media.into_parts();
        let Some(source) = source else {
            self.diagnostics.push(missing_media_source(&element));
            return Ok(None);
        };
        let (asset_ref, asset_span) = source.into_parts();
        let frozen = self
            .assets
            .get(&asset_ref)
            .ok_or_else(|| SolveError::MissingFrozenAsset(asset_ref.clone()))?;
        let Some(duration) = track.duration(frozen.metadata()) else {
            self.diagnostics.push(incompatible_media_source(
                asset_span, &asset_ref, &element, track,
            ));
            return Ok(None);
        };
        let Some((start, start_reason)) = self.prepare_delay(delay) else {
            return Ok(None);
        };
        let Some(duration) = frames_for(self.timebase, duration, asset_span, &mut self.diagnostics)
        else {
            return Ok(None);
        };
        let Some(end) = start.checked_add(duration) else {
            self.diagnostics.push(frame_overflow(asset_span));
            return Ok(None);
        };

        Ok(Some(PreparedMedia {
            element,
            asset_id: frozen.id(),
            start,
            end,
            start_reason,
        }))
    }

    fn prepare_delay(
        &mut self,
        delay: Option<Authored<Duration>>,
    ) -> Option<(FrameCount, TimingReason)> {
        let Some(delay) = delay else {
            return Some((FrameCount::ZERO, TimingReason::ShotStart));
        };
        let (delay, span) = delay.into_parts();
        let frames = frames_for(self.timebase, delay, span, &mut self.diagnostics)?;

        Some((frames, TimingReason::AuthoredDelay(span)))
    }

    fn lower_content(
        &mut self,
        content: PreparedContent,
        shot: FrameInterval,
    ) -> Option<TimelineContent> {
        match content {
            PreparedContent::Video(video) => Some(lower_video(video, shot)),
            PreparedContent::VoiceOver { media, text } => Some(lower_voice_over(media, text, shot)),
            PreparedContent::Overlay(overlay) => self.lower_overlay(overlay, shot),
        }
    }

    fn lower_overlay(
        &mut self,
        overlay: ResolvedOverlay,
        shot: FrameInterval,
    ) -> Option<TimelineContent> {
        let (element, start, text) = overlay.into_parts();
        let start = self.overlay_start(start, shot, element.span())?;

        if start.at < shot.start() || start.at >= shot.end() {
            self.diagnostics
                .push(timing_outside_shot(start.authored_at, start.at, shot));
            return None;
        }

        let timing = TimelineTiming::new(
            interval(start.at, shot.end()),
            start.reason,
            TimingReason::ShotEnd,
        );
        let overlay = TimelineOverlay::new(timeline_element(element), timing, timeline_text(text));

        Some(TimelineContent::Overlay(overlay))
    }

    fn overlay_start(
        &mut self,
        start: ResolvedStart,
        shot: FrameInterval,
        default_span: SourceSpan,
    ) -> Option<OverlayStart> {
        match start {
            ResolvedStart::ShotStart => Some(OverlayStart {
                at: shot.start(),
                reason: TimingReason::ShotStart,
                authored_at: default_span,
            }),
            ResolvedStart::Delayed(delay) => self.delayed_start(delay, shot.start()),
            ResolvedStart::Cue(event) => self.event_start(event),
        }
    }

    fn delayed_start(
        &mut self,
        delay: Authored<Duration>,
        shot_start: FrameIndex,
    ) -> Option<OverlayStart> {
        let (delay, authored_at) = delay.into_parts();
        let frames = frames_for(self.timebase, delay, authored_at, &mut self.diagnostics)?;
        let at = advance(shot_start, frames, authored_at, &mut self.diagnostics)?;

        Some(OverlayStart {
            at,
            reason: TimingReason::AuthoredDelay(authored_at),
            authored_at,
        })
    }

    fn event_start(&self, event: Authored<EventRef>) -> Option<OverlayStart> {
        let (event, authored_at) = event.into_parts();
        let EventRef::Cue(id) = &event;
        let at = self.events.get(id)?.at();

        Some(OverlayStart {
            at,
            reason: TimingReason::Event { event, authored_at },
            authored_at,
        })
    }

    fn explicit_duration(&mut self, duration: Option<Authored<Duration>>) -> ExplicitDuration {
        let Some(duration) = duration else {
            return ExplicitDuration::Absent;
        };
        let (duration, authored_at) = duration.into_parts();
        let Some(frames) = frames_for(self.timebase, duration, authored_at, &mut self.diagnostics)
        else {
            return ExplicitDuration::Rejected;
        };

        ExplicitDuration::Available {
            frames,
            authored_at,
        }
    }

    fn shot_duration(
        &mut self,
        explicit: ExplicitDuration,
        primary: PrimaryDuration,
        source: SourceSpan,
    ) -> Option<ShotDuration> {
        if let PrimaryDuration::Available(primary) = primary {
            if let ExplicitDuration::Available { authored_at, .. } = explicit {
                self.diagnostics
                    .push(conflicting_duration_sources(authored_at, primary.source));
            }
            return Some(ShotDuration::new(
                primary.frames,
                TimingReason::LongestContent(primary.source),
            ));
        }
        if let ExplicitDuration::Available {
            frames,
            authored_at,
        } = explicit
        {
            return Some(ShotDuration::new(
                frames,
                TimingReason::ExplicitDuration(authored_at),
            ));
        }
        if matches!(explicit, ExplicitDuration::Absent)
            && matches!(primary, PrimaryDuration::Absent)
        {
            self.diagnostics.push(missing_duration_source(source));
        }
        None
    }
}

/// Sequential placement state after the last trustworthy shot boundary.
///
/// `Lost` retains the last known position for enclosing spans while preventing
/// later shots from receiving invented absolute intervals.
enum PlacementCursor {
    FilmStart,
    Sequential(FrameIndex),
    Lost(FrameIndex),
}

impl PlacementCursor {
    const fn position(&self) -> FrameIndex {
        match self {
            Self::FilmStart => FrameIndex::ZERO,
            Self::Sequential(at) | Self::Lost(at) => *at,
        }
    }

    const fn scene_start(&self) -> (FrameIndex, TimingReason) {
        match self {
            Self::FilmStart => (FrameIndex::ZERO, TimingReason::FilmStart),
            Self::Sequential(at) | Self::Lost(at) => (*at, TimingReason::Sequential),
        }
    }

    const fn next_shot(&self) -> Option<(FrameIndex, TimingReason)> {
        match self {
            Self::FilmStart => Some((FrameIndex::ZERO, TimingReason::FilmStart)),
            Self::Sequential(at) => Some((*at, TimingReason::Sequential)),
            Self::Lost(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
enum MediaTrack {
    Audio,
    Video,
}

impl MediaTrack {
    fn duration(self, metadata: &AssetMetadata) -> Option<Duration> {
        match self {
            Self::Audio => metadata.audio_metadata().map(AudioMetadata::duration),
            Self::Video => metadata.video_metadata().map(VideoMetadata::duration),
        }
    }
}

/// Result of validating an authored shot duration.
///
/// `Rejected` is distinct from `Absent`: the former already emitted the useful
/// diagnostic and must not also trigger a missing-duration error.
#[derive(Clone, Copy)]
enum ExplicitDuration {
    Absent,
    Rejected,
    Available {
        frames: FrameCount,
        authored_at: SourceSpan,
    },
}

struct ShotDuration {
    frames: FrameCount,
    reason: TimingReason,
}

impl ShotDuration {
    const fn new(frames: FrameCount, reason: TimingReason) -> Self {
        Self { frames, reason }
    }
}

#[derive(Clone, Copy)]
struct PrimaryEnd {
    frames: FrameCount,
    source: SourceSpan,
}

/// Longest usable primary-content duration seen while preparing one shot.
///
/// A rejected primary source carries the same diagnostic-suppression meaning
/// as [`ExplicitDuration::Rejected`].
#[derive(Clone, Copy)]
enum PrimaryDuration {
    Absent,
    Rejected,
    Available(PrimaryEnd),
}

impl PrimaryDuration {
    fn reject(&mut self) {
        if matches!(self, Self::Absent) {
            *self = Self::Rejected;
        }
    }

    fn include(&mut self, candidate: PrimaryEnd) {
        let replace = match self {
            Self::Available(current) => candidate.frames > current.frames,
            Self::Absent | Self::Rejected => true,
        };
        if replace {
            *self = Self::Available(candidate);
        }
    }
}

/// Content facts collected before the shot itself can receive an interval.
struct PreparedShot {
    content: Vec<PreparedContent>,
    primary: PrimaryDuration,
}

enum PreparedContent {
    Video(PreparedMedia),
    VoiceOver {
        media: PreparedMedia,
        text: Vec<ResolvedText>,
    },
    Overlay(ResolvedOverlay),
}

impl PreparedContent {
    fn primary_end(&self) -> Option<PrimaryEnd> {
        let media = match self {
            Self::Video(media) | Self::VoiceOver { media, .. } => media,
            Self::Overlay(_) => return None,
        };

        Some(PrimaryEnd {
            frames: media.end,
            source: media.element.span(),
        })
    }
}

const fn is_primary_content(content: &ResolvedShotContent) -> bool {
    matches!(
        content,
        ResolvedShotContent::Video(_) | ResolvedShotContent::VoiceOver(_)
    )
}

/// Media with shot-relative bounds, before its owning shot is placed.
struct PreparedMedia {
    element: ResolvedElement,
    asset_id: FrozenAssetId,
    start: FrameCount,
    end: FrameCount,
    start_reason: TimingReason,
}

/// Media placed at absolute Timeline IR bounds.
struct PlacedMedia {
    element: TimelineElement,
    timing: TimelineTiming,
    asset_id: FrozenAssetId,
}

fn place_media(media: PreparedMedia, shot: FrameInterval) -> PlacedMedia {
    let start = shot
        .start()
        .checked_advance(media.start)
        .expect("a placed shot bounds every prepared media start");
    let end = shot
        .start()
        .checked_advance(media.end)
        .expect("a placed shot bounds every prepared media end");
    let timing = TimelineTiming::new(
        interval(start, end),
        media.start_reason,
        TimingReason::AssetDuration,
    );

    PlacedMedia {
        element: timeline_element(media.element),
        timing,
        asset_id: media.asset_id,
    }
}

fn lower_video(media: PreparedMedia, shot: FrameInterval) -> TimelineContent {
    let media = place_media(media, shot);
    let video = TimelineVideo::new(media.element, media.timing, media.asset_id);

    TimelineContent::Video(video)
}

fn lower_voice_over(
    media: PreparedMedia,
    text: Vec<ResolvedText>,
    shot: FrameInterval,
) -> TimelineContent {
    let media = place_media(media, shot);
    let text = timeline_text(text);
    let voice_over = TimelineVoiceOver::new(media.element, media.timing, media.asset_id, text);

    TimelineContent::VoiceOver(voice_over)
}

struct OverlayStart {
    at: FrameIndex,
    reason: TimingReason,
    authored_at: SourceSpan,
}

fn frame_at(
    timebase: Timebase,
    duration: Duration,
    authored_at: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<FrameIndex> {
    let Ok(frame) = timebase.frame_at(duration, Rounding::Ceil) else {
        diagnostics.push(frame_overflow(authored_at));
        return None;
    };

    Some(frame)
}

fn frames_for(
    timebase: Timebase,
    duration: Duration,
    authored_at: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<FrameCount> {
    let Ok(frames) = timebase.frames_for(duration, Rounding::Ceil) else {
        diagnostics.push(frame_overflow(authored_at));
        return None;
    };

    Some(frames)
}

fn advance(
    start: FrameIndex,
    duration: FrameCount,
    source: SourceSpan,
    diagnostics: &mut Diagnostics,
) -> Option<FrameIndex> {
    let Some(end) = start.checked_advance(duration) else {
        diagnostics.push(frame_overflow(source));
        return None;
    };

    Some(end)
}

fn timeline_element(element: ResolvedElement) -> TimelineElement {
    let (kind, id, source) = element.into_parts();
    TimelineElement::new(kind, id, source)
}

fn timeline_text(text: Vec<ResolvedText>) -> Vec<TimelineText> {
    text.into_iter()
        .map(|text| {
            let (text, source) = text.into_parts();
            TimelineText::new(text, source)
        })
        .collect()
}

fn interval(start: FrameIndex, end: FrameIndex) -> FrameInterval {
    FrameInterval::new(start, end).expect("the solver constructs ordered frame bounds")
}

fn missing_media_source(element: &ResolvedElement) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::MissingMediaSource,
        element.span(),
        format!(
            "element <{}> needs a frozen media source for timeline solving",
            element.kind()
        ),
    )
    .expect("a formatted media-source message is non-blank")
    .with_help(format!("add src=\"...\" to <{}>", element.kind()))
    .expect("a formatted media-source help is non-blank")
}

fn incompatible_media_source(
    primary: SourceSpan,
    asset: &AssetRef,
    element: &ResolvedElement,
    track: MediaTrack,
) -> Diagnostic {
    let (stream, asset_kind) = match track {
        MediaTrack::Audio => ("audio", "an audio"),
        MediaTrack::Video => ("visual", "a video"),
    };
    Diagnostic::new(
        DiagnosticCode::IncompatibleMediaSource,
        primary,
        format!(
            "<{}> source \"{asset}\" has no {stream} stream",
            element.kind()
        ),
    )
    .expect("a formatted media-track message is non-blank")
    .with_help(format!(
        "choose {asset_kind} asset for <{}>",
        element.kind()
    ))
    .expect("a formatted media-track help is non-blank")
}

fn missing_duration_source(primary: SourceSpan) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::MissingDurationSource,
        primary,
        "shot has no media or explicit duration",
    )
    .expect("the static duration-source message is non-blank")
    .with_help("add timed media content or an explicit shot duration")
    .expect("the static duration-source help is non-blank")
}

fn conflicting_duration_sources(primary: SourceSpan, media: SourceSpan) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::ConflictingDurationSources,
        primary,
        "explicit shot duration conflicts with media-derived duration",
    )
    .expect("the static duration-conflict message is non-blank")
    .with_help("remove the explicit duration or the timed media content")
    .expect("the static duration-conflict help is non-blank")
    .with_related(media, "media-derived duration is available here")
    .expect("the static duration-conflict relation is non-blank")
}

fn timing_outside_shot(primary: SourceSpan, start: FrameIndex, shot: FrameInterval) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::TimingOutsideShot,
        primary,
        format!(
            "content starts at frame {}, outside its shot interval {}..{}",
            start.get(),
            shot.start().get(),
            shot.end().get(),
        ),
    )
    .expect("a formatted timing message is non-blank")
    .with_help("use an earlier cue or delay, or extend the owning shot")
    .expect("the static timing help is non-blank")
}

fn frame_overflow(primary: SourceSpan) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::FrameConversionOverflow,
        primary,
        "time value exceeds the selected frame domain",
    )
    .expect("the static frame-domain message is non-blank")
    .with_help("use a shorter duration or a lower compile frame rate")
    .expect("the static frame-domain help is non-blank")
}
