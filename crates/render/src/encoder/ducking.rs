//! Exact music-ducking projection onto the fixed output sample grid.
//!
//! Rust first coalesces voice-over influence windows into non-overlapping gain
//! segments. `FFmpeg` then executes those facts without inspecting waveforms or
//! evaluating one growing expression for every sample.

use std::fmt::Write as _;

use onmark_core::model::{
    AudioGain, AudioSampleConversionOverflow, AudioSampleCount, AudioSampleRate, FrameCount,
    FrameIndex, FrameInterval, FrameRate, Rounding,
};
use onmark_core::timeline::TimelineAudioDucking;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AudioDuckingInput {
    target: AudioGain,
    attack: FrameCount,
    release: FrameCount,
    voice_overs: Vec<FrameInterval>,
}

impl AudioDuckingInput {
    pub(super) fn from_timeline(
        ducking: &TimelineAudioDucking,
        placement_start: FrameIndex,
    ) -> Self {
        let voice_overs = ducking
            .voice_overs()
            .iter()
            .map(|interval| rebase_interval(*interval, placement_start))
            .collect();
        Self {
            target: ducking.target(),
            attack: ducking.attack(),
            release: ducking.release(),
            voice_overs,
        }
    }

    #[cfg(test)]
    pub(super) fn fixture(
        target: AudioGain,
        attack: FrameCount,
        release: FrameCount,
        voice_overs: Vec<FrameInterval>,
    ) -> Self {
        Self {
            target,
            attack,
            release,
            voice_overs,
        }
    }
}

fn rebase_interval(interval: FrameInterval, origin: FrameIndex) -> FrameInterval {
    let start = interval
        .start()
        .get()
        .checked_sub(origin.get())
        .map(FrameIndex::new)
        .expect("solved ducking begins within its music placement");
    let end = interval
        .end()
        .get()
        .checked_sub(origin.get())
        .map(FrameIndex::new)
        .expect("solved ducking ends within its music placement");
    FrameInterval::new(start, end).expect("rebasing preserves interval order")
}

pub(super) struct AudioSampleDucking {
    target: AudioGain,
    segments: Vec<AudioGainSegment>,
}

impl AudioSampleDucking {
    pub(super) fn project(
        ducking: Option<&AudioDuckingInput>,
        base: AudioGain,
        samples: AudioSampleCount,
        frame_rate: FrameRate,
        sample_rate: AudioSampleRate,
    ) -> Result<Self, AudioSampleConversionOverflow> {
        let Some(ducking) = ducking else {
            return Ok(Self::flat());
        };
        if ducking.voice_overs.is_empty() || ducking.target == base || base.numerator() == 0 {
            return Ok(Self::flat());
        }

        let attack = sample_rate
            .samples_for(ducking.attack, frame_rate, Rounding::Ceil)?
            .get();
        let release = sample_rate
            .samples_for(ducking.release, frame_rate, Rounding::Ceil)?
            .get();
        let mut windows = Vec::with_capacity(ducking.voice_overs.len());
        for voice_over in &ducking.voice_overs {
            windows.push(sample_window(
                *voice_over,
                attack,
                release,
                samples,
                frame_rate,
                sample_rate,
            )?);
        }
        let windows = merge_windows(windows);

        Ok(Self {
            target: ducking.target,
            segments: gain_segments(&windows, samples.get()),
        })
    }

    fn flat() -> Self {
        Self {
            target: AudioGain::UNITY,
            segments: Vec::new(),
        }
    }

    pub(super) fn write_filter(
        &self,
        output: &mut String,
        index: usize,
        base: AudioGain,
        samples: AudioSampleCount,
    ) {
        if self.segments.is_empty() {
            write!(
                output,
                "[prepared{index}]volume={}/{}[leveled{index}];",
                base.numerator(),
                base.denominator(),
            )
            .expect("writing into a String cannot fail");
            return;
        }

        write!(output, "[prepared{index}]asplit={}", self.segments.len())
            .expect("writing into a String cannot fail");
        for segment in 0..self.segments.len() {
            write!(output, "[duck{index}_{segment}]").expect("writing into a String cannot fail");
        }
        output.push(';');

        for (position, segment) in self.segments.iter().enumerate() {
            segment.write_filter(output, index, position, base, self.target);
        }
        for position in 0..self.segments.len() {
            write!(output, "[gain{index}_{position}]").expect("writing into a String cannot fail");
        }
        write!(
            output,
            "concat=n={}:v=0:a=1,atrim=end_sample={}[leveled{index}];",
            self.segments.len(),
            samples.get(),
        )
        .expect("writing into a String cannot fail");
    }
}

#[derive(Clone, Copy)]
struct DuckingWindow {
    attack_start: u64,
    voice_start: u64,
    voice_end: u64,
    release_end: u64,
}

fn sample_window(
    voice_over: FrameInterval,
    attack: u64,
    release: u64,
    placement: AudioSampleCount,
    frame_rate: FrameRate,
    sample_rate: AudioSampleRate,
) -> Result<DuckingWindow, AudioSampleConversionOverflow> {
    let voice_start = sample_rate
        .samples_for(
            FrameCount::new(voice_over.start().get()),
            frame_rate,
            Rounding::Ceil,
        )?
        .get();
    let voice_end = sample_rate
        .samples_for(
            FrameCount::new(voice_over.end().get()),
            frame_rate,
            Rounding::Ceil,
        )?
        .get();
    // A release past the placement end is indistinguishable from any larger
    // value because the encoder cannot emit samples beyond that boundary.
    let release_end = voice_end
        .checked_add(release)
        .map_or(placement.get(), |end| end.min(placement.get()));

    Ok(DuckingWindow {
        attack_start: voice_start.saturating_sub(attack),
        voice_start,
        voice_end,
        release_end,
    })
}

fn merge_windows(mut windows: Vec<DuckingWindow>) -> Vec<DuckingWindow> {
    windows.sort_by_key(|window| window.attack_start);
    let mut merged: Vec<DuckingWindow> = Vec::with_capacity(windows.len());

    for window in windows {
        let Some(previous) = merged.last_mut() else {
            merged.push(window);
            continue;
        };
        if window.attack_start >= previous.release_end {
            merged.push(window);
            continue;
        }

        previous.voice_start = previous.voice_start.min(window.voice_start);
        previous.voice_end = previous.voice_end.max(window.voice_end);
        previous.release_end = previous.release_end.max(window.release_end);
    }

    merged
}

fn gain_segments(windows: &[DuckingWindow], samples: u64) -> Vec<AudioGainSegment> {
    let mut segments = Vec::with_capacity(windows.len() * 4 + 1);
    let mut cursor = 0_u64;

    for window in windows {
        push_segment(&mut segments, cursor, window.attack_start, GainPhase::Base);
        push_segment(
            &mut segments,
            window.attack_start,
            window.voice_start,
            GainPhase::Attack,
        );
        push_segment(
            &mut segments,
            window.voice_start,
            window.voice_end,
            GainPhase::Target,
        );
        push_segment(
            &mut segments,
            window.voice_end,
            window.release_end,
            GainPhase::Release,
        );
        cursor = window.release_end;
    }
    push_segment(&mut segments, cursor, samples, GainPhase::Base);

    segments
}

fn push_segment(segments: &mut Vec<AudioGainSegment>, start: u64, end: u64, phase: GainPhase) {
    if start < end {
        segments.push(AudioGainSegment { start, end, phase });
    }
}

#[derive(Clone, Copy)]
struct AudioGainSegment {
    start: u64,
    end: u64,
    phase: GainPhase,
}

impl AudioGainSegment {
    fn write_filter(
        self,
        output: &mut String,
        track: usize,
        position: usize,
        base: AudioGain,
        target: AudioGain,
    ) {
        write!(
            output,
            "[duck{track}_{position}]atrim=start_sample={}:end_sample={},asetpts=N/SR/TB,",
            self.start, self.end,
        )
        .expect("writing into a String cannot fail");

        match self.phase {
            GainPhase::Base => write_gain(output, base),
            GainPhase::Target => write_gain(output, target),
            GainPhase::Attack => write_ramp(output, base, target, self.len(), "out"),
            GainPhase::Release => write_ramp(output, base, target, self.len(), "in"),
        }
        write!(output, "[gain{track}_{position}];").expect("writing into a String cannot fail");
    }

    const fn len(self) -> u64 {
        self.end - self.start
    }
}

#[derive(Clone, Copy)]
enum GainPhase {
    Base,
    Attack,
    Target,
    Release,
}

fn write_gain(output: &mut String, gain: AudioGain) {
    write!(output, "volume={}/{}", gain.numerator(), gain.denominator())
        .expect("writing into a String cannot fail");
}

fn write_ramp(
    output: &mut String,
    base: AudioGain,
    target: AudioGain,
    samples: u64,
    direction: &str,
) {
    write_gain(output, base);
    write!(
        output,
        ",afade=t={direction}:ss=0:ns={samples}:curve=tri:\
         silence={}*{}/({}*{}):unity=1",
        target.numerator(),
        base.denominator(),
        target.denominator(),
        base.numerator(),
    )
    .expect("writing into a String cannot fail");
}

#[cfg(test)]
mod tests {
    use super::{AudioDuckingInput, AudioSampleDucking};
    use onmark_core::model::{
        AudioGain, AudioSampleCount, AudioSampleRate, FrameCount, FrameIndex, FrameInterval,
        FrameRate,
    };

    #[test]
    fn overlapping_influence_windows_remain_ducked_between_voice_overs() {
        let first = interval(15, 30);
        let second = interval(35, 45);
        let input = AudioDuckingInput::fixture(
            AudioGain::new(1, 4).expect("one quarter is valid"),
            FrameCount::new(1),
            FrameCount::new(8),
            vec![first, second],
        );
        let sample_rate = AudioSampleRate::new(48_000).expect("the sample rate is positive");
        let frame_rate = FrameRate::new(30, 1).expect("the frame rate is positive");
        let ducking = AudioSampleDucking::project(
            Some(&input),
            AudioGain::UNITY,
            AudioSampleCount::new(96_000),
            frame_rate,
            sample_rate,
        )
        .expect("the fixture fits the sample grid");
        let mut filter = String::new();

        ducking.write_filter(
            &mut filter,
            0,
            AudioGain::UNITY,
            AudioSampleCount::new(96_000),
        );

        assert!(
            filter.contains("atrim=start_sample=24000:end_sample=72000,asetpts=N/SR/TB,volume=1/4")
        );
        assert!(filter.contains("[prepared0]asplit=5"));
    }

    #[test]
    fn accepts_silence_as_an_absolute_duck_target() {
        let input = AudioDuckingInput::fixture(
            AudioGain::new(0, 1).expect("silence is valid"),
            FrameCount::new(1),
            FrameCount::new(8),
            vec![interval(15, 30)],
        );
        let ducking = project(&input, AudioSampleCount::new(48_000));
        let mut filter = String::new();

        ducking.write_filter(
            &mut filter,
            0,
            AudioGain::UNITY,
            AudioSampleCount::new(48_000),
        );

        assert!(filter.contains("silence=0*1/(1*1):unity=1"));
        assert!(filter.contains("volume=0/1"));
    }

    #[test]
    fn clips_release_at_the_music_sample_boundary() {
        let input = AudioDuckingInput::fixture(
            AudioGain::new(1, 4).expect("one quarter is valid"),
            FrameCount::new(1),
            FrameCount::new(8),
            vec![interval(15, 30)],
        );
        let samples = AudioSampleCount::new(48_000);
        let ducking = project(&input, samples);
        let mut filter = String::new();

        ducking.write_filter(&mut filter, 0, AudioGain::UNITY, samples);

        assert!(!filter.contains("atrim=start_sample=48000"));
        assert!(filter.contains("end_sample=48000"));
    }

    fn project(input: &AudioDuckingInput, samples: AudioSampleCount) -> AudioSampleDucking {
        let sample_rate = AudioSampleRate::new(48_000).expect("the sample rate is positive");
        let frame_rate = FrameRate::new(30, 1).expect("the frame rate is positive");
        AudioSampleDucking::project(
            Some(input),
            AudioGain::UNITY,
            samples,
            frame_rate,
            sample_rate,
        )
        .expect("the fixture fits the sample grid")
    }

    fn interval(start: u64, end: u64) -> FrameInterval {
        FrameInterval::new(FrameIndex::new(start), FrameIndex::new(end))
            .expect("the fixture interval is ordered")
    }
}
