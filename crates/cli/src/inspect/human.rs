//! Concise terminal rendering of solved film facts.

use std::fmt;
use std::io::{self, Write};

use onmark_core::model::FrameInterval;
use onmark_core::timeline::{
    TimelineContent, TimelineElement, TimelineIr, TimelineScene, TimelineShot, TimelineText,
    TimelineTiming, TimelineVideo,
};

use crate::check::{RegionInspection, Validation};
use crate::diagnostic;

pub(super) fn write(validation: &Validation) -> io::Result<()> {
    let report = &validation.report;
    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    drop(stderr);

    let Some(inspection) = &validation.inspection else {
        return Ok(());
    };
    let mut stdout = io::stdout().lock();
    write_timeline(&mut stdout, &inspection.timeline)?;
    writeln!(stdout, "Frozen assets: {}", inspection.assets)?;
    for (index, region) in inspection.regions.iter().enumerate() {
        write_region(&mut stdout, index, region)?;
    }
    Ok(())
}

fn write_timeline(output: &mut impl Write, timeline: &TimelineIr) -> io::Result<()> {
    let frame_rate = timeline.timebase().frame_rate();
    writeln!(
        output,
        "Timeline {} at {}/{} fps",
        Interval(timeline.interval()),
        frame_rate.numerator(),
        frame_rate.denominator(),
    )?;
    for (id, event) in timeline.events() {
        writeln!(output, "Event #{id} at {}", event.at().get())?;
    }
    for (index, scene) in timeline.scenes().iter().enumerate() {
        write_scene(output, index, scene)?;
    }
    for audio in timeline.general_audio() {
        writeln!(
            output,
            "Audio {} {} asset {}",
            audio.kind().as_str(),
            Timing(audio.timing()),
            audio.asset_id(),
        )?;
    }
    for (index, caption) in timeline.captions().iter().enumerate() {
        writeln!(
            output,
            "Caption {index} {} {:?}",
            Interval(caption.interval()),
            caption.text(),
        )?;
    }
    Ok(())
}

fn write_scene(output: &mut impl Write, index: usize, scene: &TimelineScene) -> io::Result<()> {
    writeln!(
        output,
        "Scene {index} {} {}",
        Element(scene.element()),
        Timing(scene.timing()),
    )?;
    for (index, shot) in scene.shots().iter().enumerate() {
        write_shot(output, index, shot)?;
    }
    Ok(())
}

fn write_shot(output: &mut impl Write, index: usize, shot: &TimelineShot) -> io::Result<()> {
    writeln!(
        output,
        "  Shot {index} {} {}",
        Element(shot.element()),
        Timing(shot.timing()),
    )?;
    for content in shot.content() {
        write_content(output, content)?;
    }
    Ok(())
}

fn write_content(output: &mut impl Write, content: &TimelineContent) -> io::Result<()> {
    match content {
        TimelineContent::Video(video) => write_video(output, video),
        TimelineContent::VoiceOver(voice_over) => writeln!(
            output,
            "    {} {} asset {} text {:?}",
            Element(voice_over.element()),
            Timing(voice_over.timing()),
            voice_over.asset_id(),
            text(voice_over.text()),
        ),
        TimelineContent::Overlay(overlay) => writeln!(
            output,
            "    {} {} text {:?}",
            Element(overlay.element()),
            Timing(overlay.timing()),
            text(overlay.text()),
        ),
    }
}

fn write_video(output: &mut impl Write, video: &TimelineVideo) -> io::Result<()> {
    let source = video.source();
    writeln!(
        output,
        concat!(
            "    {} {} asset {} source {}..{} of {} at {}/{}x, ",
            "plays {}, hold-last {}",
        ),
        Element(video.element()),
        Timing(video.timing()),
        video.asset_id(),
        source.interval().start(),
        source.interval().end(),
        source.natural_duration(),
        source.playback_rate().numerator(),
        source.playback_rate().denominator(),
        source.plays().get(),
        source.hold_last(),
    )
}

fn write_region(
    output: &mut impl Write,
    index: usize,
    region: &RegionInspection,
) -> io::Result<()> {
    writeln!(
        output,
        "Region {index}: evaluate {}..{}, output {}..{}, {}, {}, {} native media, bundle {}",
        region.evaluation_start,
        region.evaluation_end,
        region.output_start,
        region.output_end,
        region.visual_mode,
        region.capture_cadence,
        region.native_media,
        region.bundle_id,
    )
}

struct Element<'a>(&'a TimelineElement);

impl fmt::Display for Element<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "<{}", self.0.kind())?;
        if let Some(id) = self.0.id() {
            write!(formatter, " #{id}")?;
        }
        formatter.write_str(">")
    }
}

struct Interval(FrameInterval);

impl fmt::Display for Interval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}..{}",
            self.0.start().get(),
            self.0.end().get()
        )
    }
}

struct Timing<'a>(&'a TimelineTiming);

impl fmt::Display for Timing<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ({} -> {})",
            Interval(self.0.interval()),
            self.0.start_reason().as_str(),
            self.0.end_reason().as_str(),
        )
    }
}

fn text(runs: &[TimelineText]) -> String {
    runs.iter().map(TimelineText::text).collect()
}

#[cfg(test)]
mod tests {
    use super::write_region;
    use crate::check::RegionInspection;

    #[test]
    fn names_native_media_in_each_render_region() {
        let region = RegionInspection {
            evaluation_start: 0,
            evaluation_end: 60,
            output_start: 0,
            output_end: 60,
            visual_mode: "separableBackdrop",
            capture_cadence: "placementBounded",
            native_media: 2,
            bundle_id: "sha256:fixture".into(),
        };
        let mut output = Vec::new();

        write_region(&mut output, 0, &region).expect("the region is printable");

        assert_eq!(
            String::from_utf8(output).expect("inspection text is UTF-8"),
            concat!(
                "Region 0: evaluate 0..60, output 0..60, separableBackdrop, ",
                "placementBounded, 2 native media, bundle sha256:fixture\n",
            ),
        );
    }
}
