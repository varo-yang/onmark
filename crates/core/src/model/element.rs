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
    /// Primary video content.
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
    /// Recognizes one HTML-normalized screenplay element name.
    #[must_use]
    pub fn from_local_name(name: &str) -> Option<Self> {
        match name {
            "om-film" => Some(Self::Film),
            "om-cues" => Some(Self::Cues),
            "om-cue" => Some(Self::Cue),
            "om-scene" => Some(Self::Scene),
            "om-shot" => Some(Self::Shot),
            "video" => Some(Self::Video),
            "om-vo" => Some(Self::VoiceOver),
            "om-music" => Some(Self::Music),
            "om-sfx" => Some(Self::SoundEffect),
            "om-title" => Some(Self::Title),
            "om-cta" => Some(Self::CallToAction),
            _ => None,
        }
    }

    /// Returns the stable source spelling for this element kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Film => "om-film",
            Self::Cues => "om-cues",
            Self::Cue => "om-cue",
            Self::Scene => "om-scene",
            Self::Shot => "om-shot",
            Self::Video => "video",
            Self::VoiceOver => "om-vo",
            Self::Music => "om-music",
            Self::SoundEffect => "om-sfx",
            Self::Title => "om-title",
            Self::CallToAction => "om-cta",
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
            "om-film", "om-cues", "om-cue", "om-scene", "om-shot", "video", "om-vo", "om-music",
            "om-sfx", "om-title", "om-cta",
        ];

        for name in names {
            let kind = ElementKind::from_local_name(name)
                .expect("every screenplay element name must be recognized");
            assert_eq!(kind.as_str(), name);
        }

        assert_eq!(ElementKind::from_local_name("film"), None);
        assert_eq!(ElementKind::from_local_name("om-Film"), None);
    }
}
