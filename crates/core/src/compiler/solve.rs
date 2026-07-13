use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{
    AssetRef, CueId, Duration, EventRef, FrameCount, FrameIndex, FrameInterval, FrozenAsset,
    FrozenAssetId, Rounding, SourceSpan, Timebase,
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

struct Solver<'a> {
    assets: &'a BTreeMap<AssetRef, FrozenAsset>,
    timebase: Timebase,
    events: BTreeMap<CueId, TimelineEvent>,
    diagnostics: Diagnostics,
    cursor: FrameIndex,
    cursor_trustworthy: bool,
    emitted_shot: bool,
}

impl<'a> Solver<'a> {
    fn new(assets: &'a BTreeMap<AssetRef, FrozenAsset>, timebase: Timebase) -> Self {
        Self {
            assets,
            timebase,
            events: BTreeMap::new(),
            diagnostics: Diagnostics::new(),
            cursor: FrameIndex::ZERO,
            cursor_trustworthy: true,
            emitted_shot: false,
        }
    }

    fn solve(mut self, film: ResolvedFilm) -> Result<SolveReport, SolveError> {
        let (element, cues, scenes, _ids) = film.into_parts();
        self.solve_events(cues);

        let mut timeline_scenes = Vec::with_capacity(scenes.len());
        for scene in scenes {
            timeline_scenes.push(self.solve_scene(scene)?);
        }

        let interval = interval(FrameIndex::ZERO, self.cursor);
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
        let start = self.cursor;
        let starts_at_film = !self.emitted_shot;
        let mut timeline_shots = Vec::with_capacity(shots.len());

        for shot in shots {
            if let Some(shot) = self.solve_shot(shot)? {
                timeline_shots.push(shot);
            }
        }

        let start_reason = if starts_at_film {
            TimingReason::FilmStart
        } else {
            TimingReason::Sequential
        };
        let timing = TimelineTiming::new(
            interval(start, self.cursor),
            start_reason,
            TimingReason::Children,
        );
        let element = timeline_element(element);

        Ok(TimelineScene::new(element, timing, timeline_shots))
    }

    fn solve_shot(&mut self, shot: ResolvedShot) -> Result<Option<TimelineShot>, SolveError> {
        let (element, duration, content) = shot.into_parts();
        let source = element.span();
        let has_duration_source = duration.is_some() || has_primary_content(&content);
        let prepared = self.prepare_contents(content)?;
        let explicit = match duration {
            Some(duration) => self.explicit_duration(duration),
            None => None,
        };
        let primary_end = longest_primary(&prepared);
        let duration = self.shot_duration(explicit, primary_end, has_duration_source, source);
        let Some(timing) = self.place_shot(duration, source) else {
            return Ok(None);
        };
        let content = self.lower_contents(prepared, timing.interval());
        let element = timeline_element(element);
        let shot = TimelineShot::new(element, timing, content);

        Ok(Some(shot))
    }

    fn prepare_contents(
        &mut self,
        content: Vec<ResolvedShotContent>,
    ) -> Result<Vec<PreparedContent>, SolveError> {
        let mut prepared = Vec::with_capacity(content.len());

        for content in content {
            if let Some(content) = self.prepare_content(content)? {
                prepared.push(content);
            }
        }

        Ok(prepared)
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
        let start = self.cursor;
        let start_reason = if self.emitted_shot {
            TimingReason::Sequential
        } else {
            TimingReason::FilmStart
        };
        self.emitted_shot = true;

        let Some(duration) = duration else {
            self.cursor_trustworthy = false;
            return None;
        };
        if !self.cursor_trustworthy {
            return None;
        }
        let Some(end) = advance(start, duration.frames, source, &mut self.diagnostics) else {
            self.cursor_trustworthy = false;
            return None;
        };

        self.cursor = end;
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
        let duration = match track {
            MediaTrack::Audio => frozen.metadata().duration(),
            MediaTrack::Video => {
                let Some(video) = frozen.metadata().video_metadata() else {
                    self.diagnostics
                        .push(incompatible_video_source(asset_span, &asset_ref));
                    return Ok(None);
                };
                video.duration()
            }
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

    fn explicit_duration(&mut self, duration: Authored<Duration>) -> Option<ExplicitDuration> {
        let (duration, authored_at) = duration.into_parts();
        let frames = frames_for(self.timebase, duration, authored_at, &mut self.diagnostics)?;

        Some(ExplicitDuration {
            frames,
            authored_at,
        })
    }

    fn shot_duration(
        &mut self,
        explicit: Option<ExplicitDuration>,
        primary: Option<PrimaryEnd>,
        has_duration_source: bool,
        source: SourceSpan,
    ) -> Option<ShotDuration> {
        match (explicit, primary) {
            (Some(explicit), Some(primary)) => {
                self.diagnostics.push(conflicting_duration_sources(
                    explicit.authored_at,
                    primary.source,
                ));
                Some(ShotDuration::new(
                    primary.frames,
                    TimingReason::LongestContent(primary.source),
                ))
            }
            (Some(explicit), None) => Some(ShotDuration::new(
                explicit.frames,
                TimingReason::ExplicitDuration(explicit.authored_at),
            )),
            (None, Some(primary)) => Some(ShotDuration::new(
                primary.frames,
                TimingReason::LongestContent(primary.source),
            )),
            (None, None) if has_duration_source => None,
            (None, None) => {
                self.diagnostics.push(missing_duration_source(source));
                None
            }
        }
    }
}

#[derive(Clone, Copy)]
enum MediaTrack {
    Audio,
    Video,
}

struct ExplicitDuration {
    frames: FrameCount,
    authored_at: SourceSpan,
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

fn has_primary_content(content: &[ResolvedShotContent]) -> bool {
    content.iter().any(|content| {
        matches!(
            content,
            ResolvedShotContent::Video(_) | ResolvedShotContent::VoiceOver(_)
        )
    })
}

fn longest_primary(content: &[PreparedContent]) -> Option<PrimaryEnd> {
    let mut longest = None;

    for candidate in content.iter().filter_map(PreparedContent::primary_end) {
        if longest.is_none_or(|current: PrimaryEnd| candidate.frames > current.frames) {
            longest = Some(candidate);
        }
    }

    longest
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

fn incompatible_video_source(primary: SourceSpan, asset: &AssetRef) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IncompatibleMediaSource,
        primary,
        format!("<video> source \"{asset}\" has no visual stream"),
    )
    .expect("a formatted media-track message is non-blank")
    .with_help("choose a video asset for <video>")
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
