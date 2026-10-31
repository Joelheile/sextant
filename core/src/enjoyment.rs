//! How much the user likes a kind of work. Corrections set it and the finder scores with it.

use crate::text::text_enum;

text_enum!(Enjoyment {
    Hate => "hate",
    Dislike => "dislike",
    Neutral => "neutral",
    Unknown => "unknown",
    Like => "like",
    Love => "love",
});

impl Enjoyment {
    pub fn desire(self) -> f64 {
        match self {
            Self::Hate => 1.0,
            Self::Dislike => 0.9,
            Self::Neutral => 0.7,
            Self::Unknown => 0.6,
            Self::Like => 0.35,
            Self::Love => 0.0,
        }
    }
}
