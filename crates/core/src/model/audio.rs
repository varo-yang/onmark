//! Exact audio algebra shared by probing, compilation, and execution planning.

use std::error::Error;
use std::fmt;

use super::{FrameCount, FrameRate, Rounding};

/// Normalized channel layouts accepted by the Gate-four audio renderer.
///
/// The closed set is intentional. It keeps channel mapping deterministic and
/// rejects surround material before an external media tool can choose an
/// implicit downmix policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AudioChannelLayout {
    /// One centered source channel, duplicated into the stereo output.
    Mono,
    /// Two source channels retained as left and right.
    Stereo,
}

impl AudioChannelLayout {
    /// Returns the exact source channel count represented by this layout.
    #[must_use]
    pub const fn channels(self) -> u8 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

/// Exact linear amplitude applied to one audio placement.
///
/// The ratio remains rational until the media-process boundary. This avoids
/// making a decimal `f64` spelling part of Timeline IR or render planning.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AudioGain {
    numerator: u32,
    denominator: u32,
}

impl AudioGain {
    /// Unmodified source amplitude.
    pub const UNITY: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    /// Creates a canonical non-negative amplitude ratio.
    ///
    /// A zero numerator represents silence. Values above one amplify and may
    /// clip; authoring policy may impose a narrower range when a screenplay
    /// spelling is admitted.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidAudioGain`] when the denominator is zero.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, InvalidAudioGain> {
        if denominator == 0 {
            return Err(InvalidAudioGain::ZeroDenominator);
        }

        let divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    /// Returns the canonical numerator.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the canonical denominator.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// Parses the authored linear-gain spelling `integer%`.
    ///
    /// The screenplay surface deliberately admits neither decimals nor
    /// amplification. The returned ratio remains exact after parsing.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidAudioGain`] when the spelling is malformed or falls
    /// outside the inclusive `0%..=100%` authoring range.
    pub fn parse_percentage(source: &str) -> Result<Self, InvalidAudioGain> {
        let digits = source
            .strip_suffix('%')
            .ok_or(InvalidAudioGain::MalformedPercentage)?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(InvalidAudioGain::MalformedPercentage);
        }
        let percentage = digits
            .parse::<u32>()
            .map_err(|_| InvalidAudioGain::PercentageOutOfRange)?;
        if percentage > 100 {
            return Err(InvalidAudioGain::PercentageOutOfRange);
        }

        Self::new(percentage, 100)
    }
}

/// Reason a linear audio amplitude cannot be represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidAudioGain {
    /// A rational amplitude cannot have a zero denominator.
    ZeroDenominator,
    /// An authored percentage omits digits, `%`, or uses other characters.
    MalformedPercentage,
    /// An authored percentage exceeds the closed authoring range.
    PercentageOutOfRange,
}

impl fmt::Display for InvalidAudioGain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDenominator => formatter.write_str("audio-gain denominator cannot be zero"),
            Self::MalformedPercentage => {
                formatter.write_str("audio gain must use the exact spelling integer%")
            }
            Self::PercentageOutOfRange => {
                formatter.write_str("audio gain must be between 0% and 100%")
            }
        }
    }
}

impl Error for InvalidAudioGain {}

/// Positive number of decoded audio samples per second.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AudioSampleRate(u32);

impl AudioSampleRate {
    /// Creates a sample rate from its exact integer representation.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidAudioSampleRate`] when `samples_per_second` is zero.
    pub const fn new(samples_per_second: u32) -> Result<Self, InvalidAudioSampleRate> {
        if samples_per_second == 0 {
            return Err(InvalidAudioSampleRate);
        }
        Ok(Self(samples_per_second))
    }

    /// Returns the exact number of samples per second.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Projects a video-frame duration onto this audio sample grid.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSampleConversionOverflow`] when the result exceeds the
    /// audio accounting domain.
    pub fn samples_for(
        self,
        frames: FrameCount,
        frame_rate: FrameRate,
        rounding: Rounding,
    ) -> Result<AudioSampleCount, AudioSampleConversionOverflow> {
        let numerator =
            u128::from(frames.get()) * u128::from(frame_rate.denominator()) * u128::from(self.0);
        let denominator = u128::from(frame_rate.numerator());
        let quotient = numerator / denominator;
        let remainder = numerator % denominator;
        let samples = match rounding {
            Rounding::Floor => quotient,
            Rounding::Ceil => quotient + u128::from(remainder != 0),
        };

        u64::try_from(samples)
            .map(AudioSampleCount)
            .map_err(|_| AudioSampleConversionOverflow {
                frames,
                frame_rate,
                sample_rate: self,
            })
    }
}

/// Number of samples retained from one decoded audio stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AudioSampleCount(u64);

impl AudioSampleCount {
    /// Creates a sample count from its exact integer representation.
    #[must_use]
    pub const fn new(samples: u64) -> Self {
        Self(samples)
    }

    /// Returns the exact sample count.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Reason a sample rate cannot define an audio grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidAudioSampleRate;

impl fmt::Display for InvalidAudioSampleRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("audio sample rate must be positive")
    }
}

impl Error for InvalidAudioSampleRate {}

/// A frame duration whose sample-grid projection exceeds the audio domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSampleConversionOverflow {
    frames: FrameCount,
    frame_rate: FrameRate,
    sample_rate: AudioSampleRate,
}

impl AudioSampleConversionOverflow {
    /// Returns the frame duration that could not be represented.
    #[must_use]
    pub const fn frames(self) -> FrameCount {
        self.frames
    }

    /// Returns the frame rate used for the rejected conversion.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }

    /// Returns the audio sample grid used for the rejected conversion.
    #[must_use]
    pub const fn sample_rate(self) -> AudioSampleRate {
        self.sample_rate
    }
}

impl fmt::Display for AudioSampleConversionOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} frames at {}/{} fps exceed the {} Hz audio sample domain",
            self.frames.get(),
            self.frame_rate.numerator(),
            self.frame_rate.denominator(),
            self.sample_rate.get(),
        )
    }
}

impl Error for AudioSampleConversionOverflow {}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        AudioGain, AudioSampleRate, InvalidAudioGain, InvalidAudioSampleRate,
        greatest_common_divisor,
    };
    use crate::model::{FrameCount, FrameRate, Rounding};

    proptest! {
        #[test]
        fn canonicalizes_equally_scaled_audio_gains(
            numerator in 0_u32..=65_535,
            denominator in 1_u32..=65_535,
            scale in 1_u32..=65_535,
        ) {
            let original = AudioGain::new(numerator, denominator)
                .expect("the denominator is positive");
            let scaled = AudioGain::new(numerator * scale, denominator * scale)
                .expect("the generated products fit and keep a positive denominator");

            prop_assert_eq!(scaled, original);
            prop_assert_eq!(
                greatest_common_divisor(scaled.numerator(), scaled.denominator()),
                1,
            );
        }

        #[test]
        fn sample_rounding_brackets_one_exact_boundary(
            frames in any::<u32>(),
            frame_numerator in 1_u32..=240,
            frame_denominator in 1_u32..=1_001,
            samples_per_second in 1_u32..=192_000,
        ) {
            let rate = FrameRate::new(frame_numerator, frame_denominator)
                .expect("positive parts form a valid frame rate");
            let samples = AudioSampleRate::new(samples_per_second)
                .expect("the generated sample rate is positive");
            let frames = FrameCount::new(u64::from(frames));
            let floor = samples.samples_for(frames, rate, Rounding::Floor)
                .expect("the bounded generated projection fits");
            let ceil = samples.samples_for(frames, rate, Rounding::Ceil)
                .expect("the bounded generated projection fits");

            prop_assert!(floor <= ceil);
            prop_assert!(ceil.get() - floor.get() <= 1);
        }
    }

    #[test]
    fn canonicalizes_linear_amplitude() {
        assert_eq!(
            AudioGain::new(50, 100).expect("the denominator is positive"),
            AudioGain::new(1, 2).expect("the denominator is positive"),
        );
        assert_eq!(
            AudioGain::new(0, 100).expect("the denominator is positive"),
            AudioGain::new(0, 1).expect("the denominator is positive"),
        );
        assert_eq!(AudioGain::new(1, 0), Err(InvalidAudioGain::ZeroDenominator));
    }

    #[test]
    fn parses_the_closed_authored_gain_range() {
        assert_eq!(
            AudioGain::parse_percentage("25%").expect("25% is valid"),
            AudioGain::new(1, 4).expect("one quarter is valid"),
        );
        assert_eq!(
            AudioGain::parse_percentage("0%").expect("silence is valid"),
            AudioGain::new(0, 1).expect("zero gain is valid"),
        );
        assert_eq!(
            AudioGain::parse_percentage("100%").expect("unity is valid"),
            AudioGain::UNITY,
        );
        assert_eq!(
            AudioGain::parse_percentage("1.5%"),
            Err(InvalidAudioGain::MalformedPercentage),
        );
        assert_eq!(
            AudioGain::parse_percentage("101%"),
            Err(InvalidAudioGain::PercentageOutOfRange),
        );
    }

    #[test]
    fn projects_frame_duration_onto_the_sample_grid() {
        let samples = AudioSampleRate::new(48_000).expect("48 kHz is positive");
        let ntsc = FrameRate::new(30_000, 1_001).expect("the NTSC rate is valid");

        assert_eq!(
            samples
                .samples_for(FrameCount::new(1), ntsc, Rounding::Floor)
                .expect("one frame fits")
                .get(),
            1_601,
        );
        assert_eq!(
            samples
                .samples_for(FrameCount::new(1), ntsc, Rounding::Ceil)
                .expect("one frame fits")
                .get(),
            1_602,
        );
        assert_eq!(AudioSampleRate::new(0), Err(InvalidAudioSampleRate));
    }

    #[test]
    fn retains_every_fact_from_an_overflowed_projection() {
        let frames = FrameCount::new(u64::MAX);
        let frame_rate = FrameRate::new(1, u32::MAX).expect("both rate parts are positive");
        let sample_rate =
            AudioSampleRate::new(u32::MAX).expect("the maximum integer rate is positive");

        let error = sample_rate
            .samples_for(frames, frame_rate, Rounding::Ceil)
            .expect_err("the projection exceeds the sample-count domain");

        assert_eq!(error.frames(), frames);
        assert_eq!(error.frame_rate(), frame_rate);
        assert_eq!(error.sample_rate(), sample_rate);
    }
}
