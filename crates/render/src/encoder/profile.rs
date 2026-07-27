//! Closed final-delivery policies for visual encoding and audio muxing.
//!
//! One profile owns the codecs, pixel format, container, and staging suffix so
//! browser and layered execution cannot accidentally produce different files.

use tokio::process::Command;

use crate::AlphaMode;

/// One admitted final video-delivery profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeProfile {
    /// Compact H.264 video with AAC audio in an MP4 container.
    H264Mp4,
    /// Edit-friendly `ProRes` 4444 video with alpha and PCM audio in MOV.
    ProRes4444Mov,
}

impl EncodeProfile {
    /// Returns the canonical filename extension for this profile.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264Mp4 => "mp4",
            Self::ProRes4444Mov => "mov",
        }
    }

    /// Returns the canonical machine-readable profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H264Mp4 => "h264Mp4",
            Self::ProRes4444Mov => "proRes4444Mov",
        }
    }

    /// Returns the alpha contract required while capturing frames.
    #[must_use]
    pub const fn alpha_mode(self) -> AlphaMode {
        match self {
            Self::H264Mp4 => AlphaMode::Opaque,
            Self::ProRes4444Mov => AlphaMode::Preserve,
        }
    }

    pub(super) fn configure_video(self, command: &mut Command, encoder_threads: usize) {
        let threads = encoder_threads.to_string();
        match self {
            Self::H264Mp4 => {
                command
                    .args([
                        "-an", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-threads",
                    ])
                    .arg(threads)
                    .args(["-pix_fmt", "yuv420p"]);
            }
            Self::ProRes4444Mov => {
                command
                    .args(["-an", "-c:v", "prores_ks", "-profile:v", "4", "-threads"])
                    .arg(threads)
                    .args(["-pix_fmt", "yuva444p10le", "-alpha_bits", "16"]);
            }
        }
        self.configure_container(command);
    }

    pub(super) fn configure_audio(self, command: &mut Command) {
        match self {
            Self::H264Mp4 => {
                command.args(["-c:a", "aac"]);
            }
            Self::ProRes4444Mov => {
                command.args(["-c:a", "pcm_s24le"]);
            }
        }
        self.configure_container(command);
    }

    fn configure_container(self, command: &mut Command) {
        command.args([
            "-movflags",
            "+faststart",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            "-f",
        ]);
        command.arg(match self {
            Self::H264Mp4 => "mp4",
            Self::ProRes4444Mov => "mov",
        });
        command.arg("-n");
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use tokio::process::Command;

    use super::EncodeProfile;

    #[test]
    fn h264_profile_owns_the_compact_delivery_policy() {
        let arguments = video_arguments(EncodeProfile::H264Mp4);

        assert!(has_pair(&arguments, "-c:v", "libx264"));
        assert!(has_pair(&arguments, "-pix_fmt", "yuv420p"));
        assert!(has_pair(&arguments, "-f", "mp4"));
    }

    #[test]
    fn prores_4444_profile_owns_the_alpha_delivery_policy() {
        let profile = EncodeProfile::ProRes4444Mov;
        let arguments = video_arguments(profile);

        assert!(has_pair(&arguments, "-c:v", "prores_ks"));
        assert!(has_pair(&arguments, "-profile:v", "4"));
        assert!(has_pair(&arguments, "-pix_fmt", "yuva444p10le"));
        assert!(has_pair(&arguments, "-alpha_bits", "16"));
        assert!(has_pair(&arguments, "-f", "mov"));
        assert_eq!(profile.alpha_mode(), crate::AlphaMode::Preserve);
    }

    fn video_arguments(profile: EncodeProfile) -> Vec<OsString> {
        let mut command = Command::new("ffmpeg");
        profile.configure_video(&mut command, 4);
        command.as_std().get_args().map(OsString::from).collect()
    }

    fn has_pair(arguments: &[OsString], name: &str, value: &str) -> bool {
        arguments
            .windows(2)
            .any(|pair| pair[0] == name && pair[1] == value)
    }
}
