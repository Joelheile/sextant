//! Speech segmentation around a VAD probability stream (plan.md §4). The VAD says how likely
//! speech is in each 32 ms frame; this finds where speech starts and ends, with a hangover so a
//! pause inside a sentence does not cut it, and a cap so one segment always fits the ring.

/// Samples per VAD frame at 16 kHz.
pub const FRAME: usize = 512;

/// A frame this likely to be speech opens a segment.
const START_PROB: f32 = 0.5;
/// A frame this likely to be speech keeps one open.
const STAY_PROB: f32 = 0.35;
/// Silence that closes a segment: 512 ms.
const HANGOVER_FRAMES: u64 = 16;
/// Shorter bursts are clicks and breaths, not speech: 256 ms.
const MIN_SPEECH_FRAMES: u64 = 8;
/// Audio kept before the first and after the last voiced frame: 192 ms.
const PAD_FRAMES: u64 = 6;
/// A segment never grows past this: about 20 s.
const MAX_FRAMES: u64 = 625;

/// A span of speech in sample positions of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
}

enum State {
    Quiet,
    Speech {
        start: u64,
        last_voiced: u64,
        voiced: u64,
    },
}

pub struct Segmenter {
    frames: u64,
    /// First frame the next segment may use, so a split segment does not overlap its neighbour.
    floor: u64,
    state: State,
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Segmenter {
    pub fn new() -> Self {
        Self {
            frames: 0,
            floor: 0,
            state: State::Quiet,
        }
    }

    /// Whether a segment is open, so the ASR model should be ready.
    pub fn speaking(&self) -> bool {
        matches!(self.state, State::Speech { .. })
    }

    /// Feeds the probability of the next frame. Returns a segment when one just closed.
    pub fn push(&mut self, prob: f32) -> Option<Segment> {
        let frame = self.frames;
        self.frames += 1;
        match &mut self.state {
            State::Quiet => {
                if prob >= START_PROB {
                    self.state = State::Speech {
                        start: frame.saturating_sub(PAD_FRAMES).max(self.floor),
                        last_voiced: frame,
                        voiced: 1,
                    };
                }
                None
            }
            State::Speech {
                start,
                last_voiced,
                voiced,
            } => {
                if prob >= STAY_PROB {
                    *last_voiced = frame;
                    *voiced += 1;
                }
                let (start, last_voiced, voiced) = (*start, *last_voiced, *voiced);
                let too_long = frame + 1 - start >= MAX_FRAMES;
                if frame - last_voiced < HANGOVER_FRAMES && !too_long {
                    return None;
                }
                self.state = State::Quiet;
                self.floor = frame + 1;
                let last = if too_long {
                    frame
                } else {
                    (last_voiced + PAD_FRAMES).min(frame)
                };
                (voiced >= MIN_SPEECH_FRAMES).then_some(Segment {
                    start: start * FRAME as u64,
                    end: (last + 1) * FRAME as u64,
                })
            }
        }
    }

    /// Closes an open segment when the stream ends.
    pub fn finish(&mut self) -> Option<Segment> {
        let State::Speech {
            start,
            last_voiced,
            voiced,
        } = self.state
        else {
            return None;
        };
        self.state = State::Quiet;
        let last = (last_voiced + PAD_FRAMES).min(self.frames.saturating_sub(1));
        self.floor = self.frames;
        (voiced >= MIN_SPEECH_FRAMES).then_some(Segment {
            start: start * FRAME as u64,
            end: (last + 1) * FRAME as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Probabilities for `speech` frames of speech between `before` and `after` quiet frames.
    fn run(before: usize, speech: usize, after: usize) -> Vec<Segment> {
        let mut segmenter = Segmenter::new();
        let mut found = Vec::new();
        let probs = std::iter::repeat_n(0.02, before)
            .chain(std::iter::repeat_n(0.9, speech))
            .chain(std::iter::repeat_n(0.02, after));
        for prob in probs {
            found.extend(segmenter.push(prob));
        }
        found.extend(segmenter.finish());
        found
    }

    #[test]
    fn one_utterance_gives_one_segment_with_padding() {
        let found = run(50, 100, 40);
        assert_eq!(found.len(), 1);
        let frame = FRAME as u64;
        assert_eq!(found[0].start, (50 - PAD_FRAMES) * frame);
        assert_eq!(found[0].end, (50 + 100 + PAD_FRAMES) * frame);
    }

    #[test]
    fn a_pause_shorter_than_the_hangover_does_not_split() {
        let mut segmenter = Segmenter::new();
        let mut found = Vec::new();
        let pattern = [(0.9, 40), (0.02, 10), (0.9, 40), (0.02, 40)];
        for (prob, count) in pattern {
            for _ in 0..count {
                found.extend(segmenter.push(prob));
            }
        }
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_pause_longer_than_the_hangover_splits_in_two() {
        let mut segmenter = Segmenter::new();
        let mut found = Vec::new();
        for (prob, count) in [(0.9, 40), (0.02, 30), (0.9, 40), (0.02, 30)] {
            for _ in 0..count {
                found.extend(segmenter.push(prob));
            }
        }
        assert_eq!(found.len(), 2);
        assert!(found[0].end <= found[1].start);
    }

    #[test]
    fn short_clicks_are_not_speech() {
        assert!(run(20, 3, 60).is_empty());
    }

    #[test]
    fn a_dip_between_the_two_thresholds_keeps_the_segment_open() {
        let mut segmenter = Segmenter::new();
        let mut found = Vec::new();
        for (prob, count) in [(0.9, 30), (0.4, 40), (0.9, 30), (0.02, 40)] {
            for _ in 0..count {
                found.extend(segmenter.push(prob));
            }
        }
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn long_speech_is_cut_at_the_cap_without_overlap_or_gaps_in_the_speech() {
        let found = run(10, 2000, 40);
        assert!(found.len() >= 3, "{}", found.len());
        for pair in found.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
        for segment in &found {
            assert!((segment.end - segment.start) / FRAME as u64 <= MAX_FRAMES + 1);
        }
    }

    #[test]
    fn finish_closes_an_open_segment_and_speaking_reports_it() {
        let mut segmenter = Segmenter::new();
        assert!(!segmenter.speaking());
        for _ in 0..30 {
            segmenter.push(0.9);
        }
        assert!(segmenter.speaking());
        assert!(segmenter.finish().is_some());
        assert!(!segmenter.speaking());
        assert!(segmenter.finish().is_none());
    }
}
