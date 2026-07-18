//! Pure admission of typed general-audio intent into solved Timeline facts.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::model::{
    AssetRef, AudioGain, Duration, FrameConversionOverflow, FrameInterval, FrozenAsset, Rounding,
    SourceSpan,
};
use crate::timeline::{TimelineAudio, TimelineAudioKind, TimelineIr, TimelineTiming, TimingReason};

/// Semantic role selected for one non-narrative audio placement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralAudioKind {
    /// A musical bed or cue.
    Music,
    /// A discrete authored sound effect.
    SoundEffect,
}

/// Typed source intent for one general-audio placement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralAudioPlacement {
    source: AssetRef,
    start: Duration,
    end: Duration,
    gain: AudioGain,
    kind: GeneralAudioKind,
    authored_at: SourceSpan,
}

impl GeneralAudioPlacement {
    /// Creates a positive half-open placement on the film clock.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidGeneralAudioPlacement`] when the end does not follow
    /// the start.
    pub fn new(
        source: AssetRef,
        start: Duration,
        end: Duration,
        gain: AudioGain,
        kind: GeneralAudioKind,
        authored_at: SourceSpan,
    ) -> Result<Self, InvalidGeneralAudioPlacement> {
        if end.as_nanos() <= start.as_nanos() {
            return Err(InvalidGeneralAudioPlacement);
        }
        Ok(Self {
            source,
            start,
            end,
            gain,
            kind,
            authored_at,
        })
    }

    /// Returns the portable authored artifact reference.
    #[must_use]
    pub const fn source(&self) -> &AssetRef {
        &self.source
    }
}

/// Attaches typed general audio to one solved Timeline.
///
/// Projection happens once on the same frame grid as screenplay content. The
/// selected source stream must cover the requested placement duration, and the
/// placement must remain inside the solved film.
///
/// # Errors
///
/// Returns [`GeneralAudioImportError`] when frozen facts are absent or
/// incompatible, source audio is too short, timing leaves the film, or exact
/// frame projection overflows.
pub fn import_general_audio(
    mut timeline: TimelineIr,
    placements: impl IntoIterator<Item = GeneralAudioPlacement>,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
) -> Result<TimelineIr, GeneralAudioImportError> {
    let mut audio = Vec::new();
    for placement in placements {
        audio.push(project_audio(&timeline, placement, assets)?);
    }
    timeline.replace_general_audio(audio);
    Ok(timeline)
}

fn project_audio(
    timeline: &TimelineIr,
    placement: GeneralAudioPlacement,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
) -> Result<TimelineAudio, GeneralAudioImportError> {
    let GeneralAudioPlacement {
        source,
        start,
        end,
        gain,
        kind,
        authored_at,
    } = placement;
    let Some(frozen) = assets.get(&source) else {
        return Err(GeneralAudioImportError::MissingFrozenAsset(source));
    };
    let Some(metadata) = frozen.metadata().audio_metadata() else {
        return Err(GeneralAudioImportError::MissingAudioStream(source));
    };
    let duration = Duration::from_nanos(end.as_nanos() - start.as_nanos());
    if duration > metadata.duration() {
        return Err(GeneralAudioImportError::SourceTooShort(source));
    }

    let timebase = timeline.timebase();
    let start = timebase.frame_at(start, Rounding::Ceil)?;
    let end = timebase.frame_at(end, Rounding::Ceil)?;
    let interval = FrameInterval::new(start, end)
        .expect("positive authored audio times remain ordered after ceiling projection");
    if interval.is_empty() || !timeline.interval().contains_interval(interval) {
        return Err(GeneralAudioImportError::OutsideTimeline(authored_at));
    }

    let kind = match kind {
        GeneralAudioKind::Music => TimelineAudioKind::Music,
        GeneralAudioKind::SoundEffect => TimelineAudioKind::SoundEffect,
    };
    let timing = TimelineTiming::new(
        interval,
        TimingReason::ExplicitDuration(authored_at),
        TimingReason::ExplicitDuration(authored_at),
    );
    Ok(TimelineAudio::new(
        authored_at,
        timing,
        frozen.id(),
        gain,
        kind,
    ))
}

/// Reason typed general-audio intent cannot enter Timeline IR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeneralAudioImportError {
    /// The catalog omits the referenced immutable asset.
    MissingFrozenAsset(AssetRef),
    /// The referenced artifact exposes no audio stream.
    MissingAudioStream(AssetRef),
    /// The selected source stream is shorter than the requested placement.
    SourceTooShort(AssetRef),
    /// The projected placement is empty or leaves the solved film.
    OutsideTimeline(SourceSpan),
    /// Exact time cannot fit on the selected frame grid.
    FrameConversion(FrameConversionOverflow),
}

impl fmt::Display for GeneralAudioImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrozenAsset(source) => {
                write!(formatter, "audio asset {source} is missing")
            }
            Self::MissingAudioStream(source) => {
                write!(formatter, "audio asset {source} contains no audio stream")
            }
            Self::SourceTooShort(source) => {
                write!(
                    formatter,
                    "audio asset {source} is shorter than its placement"
                )
            }
            Self::OutsideTimeline(_) => {
                formatter.write_str("general audio placement lies outside the solved film")
            }
            Self::FrameConversion(source) => source.fmt(formatter),
        }
    }
}

impl Error for GeneralAudioImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FrameConversion(source) => Some(source),
            _ => None,
        }
    }
}

impl From<FrameConversionOverflow> for GeneralAudioImportError {
    fn from(source: FrameConversionOverflow) -> Self {
        Self::FrameConversion(source)
    }
}

/// A general-audio interval whose end does not follow its start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidGeneralAudioPlacement;

impl fmt::Display for InvalidGeneralAudioPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("general audio end must be after its start")
    }
}

impl Error for InvalidGeneralAudioPlacement {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::compiler;
    use crate::model::{
        AssetMetadata, AssetRef, AudioChannelLayout, AudioGain, AudioSampleRate, ByteOffset,
        Duration, FrameRate, FrozenAsset, FrozenAssetId, SourceId, SourceSpan, Timebase,
    };
    use crate::timeline::TimelineAudioKind;

    use super::{
        GeneralAudioImportError, GeneralAudioKind, GeneralAudioPlacement, import_general_audio,
    };

    #[test]
    fn imports_cross_shot_audio_as_one_absolute_timeline_fact() {
        let (timeline, assets, source) = fixture();
        let placement = GeneralAudioPlacement::new(
            source,
            Duration::parse("500ms").expect("the start is valid"),
            Duration::parse("1500ms").expect("the end is valid"),
            AudioGain::new(1, 2).expect("one half is a valid gain"),
            GeneralAudioKind::Music,
            span(),
        )
        .expect("the placement is positive");

        let timeline = import_general_audio(timeline, [placement], &assets)
            .expect("the fixture audio enters Timeline IR");
        let audio = timeline.audio().next().expect("one general track exists");

        assert_eq!(audio.timing().interval().start().get(), 15);
        assert_eq!(audio.timing().interval().end().get(), 45);
        assert_eq!(audio.gain(), AudioGain::new(1, 2).expect("gain is valid"));
        assert_eq!(audio.kind(), TimelineAudioKind::Music);
    }

    #[test]
    fn rejects_a_placement_longer_than_the_selected_stream() {
        let (timeline, assets, source) = fixture();
        let placement = GeneralAudioPlacement::new(
            source.clone(),
            Duration::ZERO,
            Duration::parse("1500ms").expect("the end is valid"),
            AudioGain::UNITY,
            GeneralAudioKind::SoundEffect,
            span(),
        )
        .expect("the placement is positive");

        assert_eq!(
            import_general_audio(timeline, [placement], &assets),
            Err(GeneralAudioImportError::SourceTooShort(source)),
        );
    }

    fn fixture() -> (
        crate::timeline::TimelineIr,
        BTreeMap<AssetRef, FrozenAsset>,
        AssetRef,
    ) {
        let parsed = compiler::parse(
            SourceId::new(0),
            r#"<film><scene><shot duration="1s" /><shot duration="1s" /></scene></film>"#,
        );
        let (document, diagnostics) = parsed.into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::resolve(film.expect("the film binds")).into_parts();
        assert!(diagnostics.is_empty());
        let rate = FrameRate::new(30, 1).expect("30 fps is valid");
        let report = compiler::solve(
            film.expect("the film resolves"),
            &BTreeMap::new(),
            Timebase::new(rate),
        )
        .expect("the screenplay references no assets");
        let timeline = report.into_parts().0.expect("the film solves");

        let source = AssetRef::parse("music.wav").expect("the fixture path is portable");
        let sample_rate = AudioSampleRate::new(44_100).expect("44.1 kHz is valid");
        let metadata = AssetMetadata::audio(
            Duration::parse("1s").expect("the source duration is valid"),
            sample_rate,
            AudioChannelLayout::Stereo,
        );
        let frozen = FrozenAsset::new(FrozenAssetId::from_sha256([9; 32]), metadata);
        let assets = BTreeMap::from([(source.clone(), frozen)]);
        (timeline, assets, source)
    }

    fn span() -> SourceSpan {
        SourceSpan::new(SourceId::new(2), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal fixture bounds are ordered")
    }
}
