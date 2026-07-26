//! Closed final-delivery policies for visual encoding and audio muxing.
//!
//! One profile owns the codecs, pixel format, container, and staging suffix so
//! browser and layered execution cannot accidentally produce different files.

use tokio::process::Command;

/// One admitted final video-delivery profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeProfile {
    /// Compact H.264 video with AAC audio in an MP4 container.
    H264Mp4,
    /// Edit-friendly `ProRes` 422 HQ video with PCM audio in a MOV container.
    ProResMov,
}

impl EncodeProfile {
    /// Returns the canonical filename extension for this profile.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264Mp4 => "mp4",
            Self::ProResMov => "mov",
        }
    }

    /// Returns the canonical machine-readable profile spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H264Mp4 => "h264Mp4",
            Self::ProResMov => "proResMov",
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
            Self::ProResMov => {
                command
                    .args(["-an", "-c:v", "prores_ks", "-profile:v", "3", "-threads"])
                    .arg(threads)
                    .args(["-pix_fmt", "yuv422p10le"]);
            }
        }
        self.configure_container(command);
    }

    pub(super) fn configure_audio(self, command: &mut Command) {
        match self {
            Self::H264Mp4 => {
                command.args(["-c:a", "aac"]);
            }
            Self::ProResMov => {
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
            Self::ProResMov => "mov",
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
    fn prores_profile_owns_the_editing_delivery_policy() {
        let arguments = video_arguments(EncodeProfile::ProResMov);

        assert!(has_pair(&arguments, "-c:v", "prores_ks"));
        assert!(has_pair(&arguments, "-profile:v", "3"));
        assert!(has_pair(&arguments, "-pix_fmt", "yuv422p10le"));
        assert!(has_pair(&arguments, "-f", "mov"));
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
