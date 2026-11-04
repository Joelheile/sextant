//! When to look: a focus change after 300 ms of settling, one second after the last
//! input, every 10 s while active, and never while idle, locked or asleep. The caller samples the
//! OS and passes time in, so this is deterministic.

/// Focus must hold this long before it counts as a change.
pub const SETTLE_MS: i64 = 300;
/// Seconds without input before the screen counts as settled.
pub const INPUT_PAUSE_S: f64 = 1.0;
pub const TICK_MS: i64 = 10_000;
/// Idle longer than this means the user is away.
pub const IDLE_LIMIT_S: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub now_ms: i64,
    /// Identifies the focused app. Any change counts as a focus change.
    pub focus: u64,
    /// Seconds since the last input event, from the OS.
    pub idle_s: f64,
    pub locked: bool,
    pub asleep: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Focus,
    InputPause,
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Away {
    Idle,
    Locked,
    Asleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Time to look at the focused window.
    Look(Reason),
    /// The user just went away. Sent once, then silence until they return.
    Left(Away),
}

#[derive(Debug, Default)]
pub struct Triggers {
    focus: Option<u64>,
    settle_at: Option<i64>,
    input_pending: bool,
    last_look: i64,
    away: Option<Away>,
}

impl Triggers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn away(&self) -> Option<Away> {
        self.away
    }

    pub fn step(&mut self, sample: Sample) -> Option<Signal> {
        let now = sample.now_ms;
        if self.focus.is_none() {
            self.last_look = now;
        }
        if self.focus != Some(sample.focus) {
            self.focus = Some(sample.focus);
            self.settle_at = Some(now + SETTLE_MS);
        }

        if let Some(kind) = away(&sample) {
            self.input_pending = false;
            self.settle_at = None;
            self.last_look = now;
            return match self.away.replace(kind) {
                None => Some(Signal::Left(kind)),
                Some(_) => None,
            };
        }
        if self.away.take().is_some() {
            self.settle_at = Some(now + SETTLE_MS);
        }

        if sample.idle_s < INPUT_PAUSE_S {
            self.input_pending = true;
        }
        let reason = if self.settle_at.is_some_and(|at| now >= at) {
            self.settle_at = None;
            Some(Reason::Focus)
        } else if self.input_pending && sample.idle_s >= INPUT_PAUSE_S {
            Some(Reason::InputPause)
        } else if now - self.last_look >= TICK_MS {
            Some(Reason::Tick)
        } else {
            None
        };
        if reason.is_some() {
            self.input_pending = false;
            self.last_look = now;
        }
        reason.map(Signal::Look)
    }
}

fn away(sample: &Sample) -> Option<Away> {
    if sample.asleep {
        Some(Away::Asleep)
    } else if sample.locked {
        Some(Away::Locked)
    } else if sample.idle_s > IDLE_LIMIT_S {
        Some(Away::Idle)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples every 250 ms. `idle` tells how many seconds the user has been idle at that moment.
    struct Run {
        triggers: Triggers,
        now_ms: i64,
        looks: Vec<(i64, Reason)>,
        left: Vec<(i64, Away)>,
    }

    impl Run {
        fn new() -> Self {
            Self {
                triggers: Triggers::new(),
                now_ms: 0,
                looks: Vec::new(),
                left: Vec::new(),
            }
        }

        fn until(&mut self, end_ms: i64, focus: u64, idle: impl Fn(i64) -> f64) {
            self.until_with(end_ms, focus, idle, false, false);
        }

        fn until_with(
            &mut self,
            end_ms: i64,
            focus: u64,
            idle: impl Fn(i64) -> f64,
            locked: bool,
            asleep: bool,
        ) {
            while self.now_ms < end_ms {
                let signal = self.triggers.step(Sample {
                    now_ms: self.now_ms,
                    focus,
                    idle_s: idle(self.now_ms),
                    locked,
                    asleep,
                });
                match signal {
                    Some(Signal::Look(reason)) => self.looks.push((self.now_ms, reason)),
                    Some(Signal::Left(kind)) => self.left.push((self.now_ms, kind)),
                    None => {}
                }
                self.now_ms += 250;
            }
        }
    }

    /// Input stopped at `stopped_ms`; idle grows from there.
    fn idle_since(stopped_ms: i64) -> impl Fn(i64) -> f64 {
        move |now| ((now - stopped_ms).max(0)) as f64 / 1000.0
    }

    #[test]
    fn a_focus_change_fires_once_after_it_settles() {
        let mut run = Run::new();
        run.until(2_000, 1, idle_since(0));
        run.looks.clear();
        // Cmd-Tab through two apps within 300 ms: only the last one counts.
        run.until(2_250, 2, idle_since(2_000));
        run.until(3_500, 3, idle_since(2_000));
        assert_eq!(run.looks, [(2_750, Reason::Focus)]);
    }

    #[test]
    fn typing_fires_one_second_after_the_last_key() {
        let mut run = Run::new();
        run.until(1_000, 1, idle_since(0));
        run.looks.clear();
        // Typing until 5 s: idle stays below 1 s, so nothing fires.
        run.until(5_000, 1, |now| (now % 500) as f64 / 1000.0);
        assert!(run.looks.is_empty(), "{:?}", run.looks);
        run.until(8_000, 1, idle_since(5_000));
        assert_eq!(run.looks, [(6_000, Reason::InputPause)]);
    }

    #[test]
    fn it_ticks_every_ten_seconds_while_active() {
        let mut run = Run::new();
        // Reading: input stopped at 0, idle stays under 60 s the whole time.
        run.until(35_000, 1, |_| 30.0);
        let ticks: Vec<i64> = run
            .looks
            .iter()
            .filter(|l| l.1 == Reason::Tick)
            .map(|l| l.0)
            .collect();
        assert_eq!(ticks, [10_500, 20_500, 30_500]);
    }

    #[test]
    fn nothing_fires_while_idle_locked_or_asleep() {
        let mut run = Run::new();
        run.until(1_000, 1, idle_since(0));
        run.looks.clear();
        run.until(120_000, 1, |now| now as f64 / 1000.0);
        let after: Vec<_> = run.looks.iter().filter(|l| l.0 > 60_250).collect();
        assert!(after.is_empty(), "{after:?}");
        assert_eq!(run.left, [(60_250, Away::Idle)]);

        let mut run = Run::new();
        run.until(1_000, 1, |_| 0.0);
        run.looks.clear();
        run.until_with(60_000, 1, |_| 0.0, true, false);
        assert!(run.looks.is_empty());
        assert_eq!(run.left, [(1_000, Away::Locked)]);

        let mut run = Run::new();
        run.until(1_000, 1, |_| 0.0);
        run.looks.clear();
        run.until_with(60_000, 1, |_| 0.0, true, true);
        assert!(run.looks.is_empty());
        assert_eq!(run.left, [(1_000, Away::Asleep)]);
    }

    #[test]
    fn coming_back_looks_once_the_focus_has_settled() {
        let mut run = Run::new();
        run.until(1_000, 1, |_| 0.0);
        run.until_with(10_000, 1, |_| 0.0, true, false);
        run.looks.clear();
        run.until(12_000, 1, |_| 0.0);
        assert_eq!(run.looks[0], (10_500, Reason::Focus));
    }
}
