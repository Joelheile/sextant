//! Utterance assembly (plan.md §4): the mic stream is "me" and the system stream is "them", so
//! the speaker is known without diarization.

use super::RATE;
use super::segmenter::Segment;
use crate::model::{Speaker, Utterance};

/// The record for `text` spoken in `segment` of a stream that started at `stream_start_s` (Unix
/// seconds). `None` when the model heard nothing worth keeping.
pub fn utterance(
    source: Speaker,
    stream_start_s: i64,
    segment: Segment,
    text: &str,
) -> Option<Utterance> {
    let text = text.trim();
    if !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    let second = |sample: u64| stream_start_s + (sample / u64::from(RATE)) as i64;
    Some(Utterance {
        ts_start: second(segment.start),
        ts_end: second(segment.end),
        source,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_follow_the_stream_start_and_the_source_is_kept() {
        let segment = Segment {
            start: 3 * 16_000 + 100,
            end: 9 * 16_000,
        };
        let me =
            utterance(Speaker::Mic, 1_000, segment, "  Ich schicke die Rechnung. ").expect("kept");
        assert_eq!((me.ts_start, me.ts_end), (1_003, 1_009));
        assert_eq!(me.source, Speaker::Mic);
        assert_eq!(me.text, "Ich schicke die Rechnung.");
        let them = utterance(Speaker::System, 1_000, segment, "ok").expect("kept");
        assert_eq!(them.source, Speaker::System);
    }

    #[test]
    fn silence_and_punctuation_only_are_dropped() {
        let segment = Segment {
            start: 0,
            end: 16_000,
        };
        for text in ["", "   ", ".", "- -", "…"] {
            assert!(
                utterance(Speaker::Mic, 0, segment, text).is_none(),
                "{text:?}"
            );
        }
    }
}
