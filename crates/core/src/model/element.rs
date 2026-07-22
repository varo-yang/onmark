//! Closed screenplay element vocabulary shared by compiler phases.

use std::fmt;

/// Closed current screenplay vocabulary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ElementKind {
    /// Root time domain of one screenplay.
    Film,
    /// Optional container for absolute cue declarations.
    Cues,
    /// Named absolute time event.
    Cue,
    /// Sequential narrative container.
    Scene,
    /// Sequential unit with one local time origin.
    Shot,
    /// Gate-one primary video content.
    Video,
    /// Authored voice-over inscription and media reference.
    VoiceOver,
    /// Film-wide musical content.
    Music,
    /// Shot-local sound effect.
    SoundEffect,
    /// Title overlay owned by one shot.
    Title,
    /// Call-to-action overlay owned by one shot.
    CallToAction,
}

/// Closed authored roles for general audio outside voice-over content.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GeneralAudioKind {
    /// Film-wide musical content.
    Music,
    /// Shot-local sound effect.
    SoundEffect,
}

impl GeneralAudioKind {
    /// Returns the screenplay element represented by this audio role.
    #[must_use]
    pub const fn element_kind(self) -> ElementKind {
        match self {
            Self::Music => ElementKind::Music,
            Self::SoundEffect => ElementKind::SoundEffect,
        }
    }
}

impl ElementKind {
    /// Recognizes an unqualified, case-sensitive screenplay element name.
    #[must_use]
    pub fn from_local_name(name: &str) -> Option<Self> {
        match name {
            "film" => Some(Self::Film),
            "cues" => Some(Self::Cues),
            "cue" => Some(Self::Cue),
            "scene" => Some(Self::Scene),
            "shot" => Some(Self::Shot),
            "video" => Some(Self::Video),
            "vo" => Some(Self::VoiceOver),
            "music" => Some(Self::Music),
            "sfx" => Some(Self::SoundEffect),
            "title" => Some(Self::Title),
            "cta" => Some(Self::CallToAction),
            _ => None,
        }
    }

    /// Returns the stable source spelling for this element kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Film => "film",
            Self::Cues => "cues",
            Self::Cue => "cue",
            Self::Scene => "scene",
            Self::Shot => "shot",
            Self::Video => "video",
            Self::VoiceOver => "vo",
            Self::Music => "music",
            Self::SoundEffect => "sfx",
            Self::Title => "title",
            Self::CallToAction => "cta",
        }
    }
}

impl fmt::Display for ElementKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::ElementKind;

    #[test]
    fn recognizes_the_closed_screenplay_vocabulary() {
        let names = [
            "film", "cues", "cue", "scene", "shot", "video", "vo", "music", "sfx", "title", "cta",
        ];

        for name in names {
            let kind = ElementKind::from_local_name(name)
                .expect("every screenplay element name must be recognized");
            assert_eq!(kind.as_str(), name);
        }

        assert_eq!(ElementKind::from_local_name("audio"), None);
        assert_eq!(ElementKind::from_local_name("Film"), None);
    }
}
