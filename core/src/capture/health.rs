//! Capture health: capture that stops while the app runs has to say so, instead of
//! looking fine. Each signal comes from the step that would fail, not from the process being up.

/// The capture thread polls every second at most, so this long without a poll means it hangs.
pub const STOPPED_AFTER_S: i64 = 30;
/// Screenshots or database writes failing this long count as stalled, not as a hiccup.
pub const FAILING_AFTER_S: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stall {
    /// The capture thread stopped polling.
    Stopped,
    /// Every screenshot fails.
    Reads,
    /// Captured records cannot be written to the database.
    Writes,
}

impl Stall {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Reads => "reads fail",
            Self::Writes => "writes fail",
        }
    }
}

/// `beat` is the last poll, the others say since when reads or writes fail.
pub fn stall(
    now: i64,
    beat: i64,
    reads_failing_since: Option<i64>,
    writes_failing_since: Option<i64>,
) -> Option<Stall> {
    let too_long = |since: Option<i64>| since.is_some_and(|since| now - since >= FAILING_AFTER_S);
    if now - beat >= STOPPED_AFTER_S {
        Some(Stall::Stopped)
    } else if too_long(writes_failing_since) {
        Some(Stall::Writes)
    } else if too_long(reads_failing_since) {
        Some(Stall::Reads)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_failures_are_hiccups_and_long_ones_stall() {
        let now = 10_000;
        assert_eq!(stall(now, now - 1, None, None), None);
        assert_eq!(stall(now, now - 1, Some(now - 60), Some(now - 60)), None);
        assert_eq!(
            stall(now, now - STOPPED_AFTER_S, None, None),
            Some(Stall::Stopped)
        );
        assert_eq!(
            stall(now, now, Some(now - FAILING_AFTER_S), None),
            Some(Stall::Reads)
        );
        assert_eq!(
            stall(
                now,
                now,
                Some(now - FAILING_AFTER_S),
                Some(now - FAILING_AFTER_S)
            ),
            Some(Stall::Writes)
        );
    }
}
