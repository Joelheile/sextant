//! The ASR model lifecycle: loaded on first speech, unloaded after 5 minutes without
//! any, so the model's memory is only held while someone talks. Time is passed in.

/// Silence after which the model is unloaded.
pub const UNLOAD_AFTER_MS: i64 = 5 * 60 * 1000;
/// A failed load is not retried sooner than this.
const RETRY_AFTER_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Load the model now, then call [`Lifecycle::loaded`] or [`Lifecycle::load_failed`].
    Load,
    Unload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Unloaded,
    Loaded { last_speech_ms: i64 },
    Failed { retry_at_ms: i64 },
}

#[derive(Debug)]
pub struct Lifecycle {
    state: State,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Lifecycle {
    pub fn new() -> Self {
        Self {
            state: State::Unloaded,
        }
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self.state, State::Loaded { .. })
    }

    /// Speech is going on. Call it for every frame of speech: it only asks for a load once.
    pub fn speech(&mut self, now_ms: i64) -> Action {
        match &mut self.state {
            State::Loaded { last_speech_ms } => {
                *last_speech_ms = now_ms;
                Action::None
            }
            State::Unloaded => Action::Load,
            State::Failed { retry_at_ms } if now_ms >= *retry_at_ms => Action::Load,
            State::Failed { .. } => Action::None,
        }
    }

    pub fn loaded(&mut self, now_ms: i64) {
        self.state = State::Loaded {
            last_speech_ms: now_ms,
        };
    }

    pub fn load_failed(&mut self, now_ms: i64) {
        self.state = State::Failed {
            retry_at_ms: now_ms + RETRY_AFTER_MS,
        };
    }

    /// Call regularly. Asks for an unload once the silence is long enough.
    pub fn tick(&mut self, now_ms: i64) -> Action {
        match self.state {
            State::Loaded { last_speech_ms } if now_ms - last_speech_ms >= UNLOAD_AFTER_MS => {
                self.state = State::Unloaded;
                Action::Unload
            }
            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_on_first_speech_and_not_before() {
        let mut lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.tick(1_000), Action::None);
        assert!(!lifecycle.is_loaded());
        assert_eq!(lifecycle.speech(2_000), Action::Load);
        lifecycle.loaded(2_500);
        assert!(lifecycle.is_loaded());
        assert_eq!(lifecycle.speech(3_000), Action::None);
    }

    #[test]
    fn unloads_after_five_minutes_without_speech() {
        let mut lifecycle = Lifecycle::new();
        lifecycle.speech(0);
        lifecycle.loaded(0);
        assert_eq!(lifecycle.tick(UNLOAD_AFTER_MS - 1), Action::None);
        assert_eq!(lifecycle.tick(UNLOAD_AFTER_MS), Action::Unload);
        assert!(!lifecycle.is_loaded());
        assert_eq!(lifecycle.tick(UNLOAD_AFTER_MS + 1), Action::None);
    }

    #[test]
    fn speech_pushes_the_unload_back_and_a_new_word_reloads() {
        let mut lifecycle = Lifecycle::new();
        lifecycle.loaded(0);
        lifecycle.speech(200_000);
        assert_eq!(lifecycle.tick(UNLOAD_AFTER_MS), Action::None);
        assert_eq!(lifecycle.tick(200_000 + UNLOAD_AFTER_MS), Action::Unload);
        assert_eq!(lifecycle.speech(700_000), Action::Load);
    }

    #[test]
    fn a_failed_load_waits_before_the_next_try() {
        let mut lifecycle = Lifecycle::new();
        assert_eq!(lifecycle.speech(0), Action::Load);
        lifecycle.load_failed(1_000);
        assert_eq!(lifecycle.speech(2_000), Action::None);
        assert_eq!(lifecycle.speech(30_999), Action::None);
        assert_eq!(lifecycle.speech(31_000), Action::Load);
    }
}
