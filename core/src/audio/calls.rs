//! Call detection (plan.md §3): a call is on when another process, not Sextant, has mic input
//! running and it is a known call app or one the user marked. Browsers count as Meet, Teams and
//! Webex in a tab, because a process cannot tell which tab holds the mic.

/// Bundle ids of the known call apps. A helper process (`<id>.helper`) belongs to its app.
pub const CALL_APPS: [&str; 17] = [
    "us.zoom.xos",
    "com.microsoft.teams",
    "com.microsoft.teams2",
    "com.tinyspeck.slackmacgap",
    "com.apple.FaceTime",
    "Cisco-Systems.Spark",
    "com.cisco.webexmeetingsapp",
    "com.webex.meetingmanager",
    "com.hnc.Discord",
    "com.google.Chrome",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "company.thebrowser.Browser",
    "com.apple.Safari",
    "org.mozilla.firefox",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
];

/// A process that has mic input running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicUser {
    pub pid: i32,
    pub bundle_id: String,
    pub name: String,
}

/// The app a call runs in. `app_id` is the bundle id of the app, without helper suffixes, and
/// `pids` are its processes that have the mic running: the ones whose sound is the other side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub app_id: String,
    pub name: String,
    pub pids: Vec<i32>,
}

/// Whether `bundle_id` is `app_id` or one of its helper processes.
pub fn belongs(app_id: &str, bundle_id: &str) -> bool {
    bundle_id
        .strip_prefix(app_id)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The call among the processes using the mic. `marked` holds the bundle ids the user marked as
/// call apps. Sextant's own pid never counts.
pub fn detect(users: &[MicUser], own_pid: i32, marked: &[String]) -> Option<Call> {
    users
        .iter()
        .filter(|user| user.pid != own_pid)
        .find_map(|user| {
            let known = CALL_APPS
                .iter()
                .copied()
                .chain(marked.iter().map(String::as_str))
                .find(|app| belongs(app, &user.bundle_id))?;
            let pids = users
                .iter()
                .filter(|other| other.pid != own_pid && belongs(known, &other.bundle_id))
                .map(|other| other.pid)
                .collect();
            Some(Call {
                app_id: known.to_string(),
                name: user.name.clone(),
                pids,
            })
        })
}

/// The mic has to be in use this long before a call starts, so a beep does not open streams.
pub const START_AFTER_MS: i64 = 2_000;
/// The mic has to be quiet this long before a call ends, so a mute button does not end it.
pub const END_AFTER_MS: i64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Started(Call),
    Ended,
}

/// Turns the noisy "who uses the mic right now" answer into call starts and ends.
#[derive(Default)]
pub struct CallTracker {
    active: Option<Call>,
    candidate: Option<(Call, i64)>,
    quiet_since: Option<i64>,
}

impl CallTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> Option<&Call> {
        self.active.as_ref()
    }

    /// One poll. `seen` is the result of [`detect`]; `now_ms` is a steady clock.
    pub fn step(&mut self, now_ms: i64, seen: Option<Call>) -> Option<Change> {
        let Some(active) = &self.active else {
            return self.maybe_start(now_ms, seen);
        };
        if seen
            .as_ref()
            .is_some_and(|call| call.app_id == active.app_id)
        {
            self.quiet_since = None;
            return None;
        }
        let since = *self.quiet_since.get_or_insert(now_ms);
        (now_ms - since >= END_AFTER_MS).then(|| self.reset())?
    }

    fn maybe_start(&mut self, now_ms: i64, seen: Option<Call>) -> Option<Change> {
        let Some(call) = seen else {
            self.candidate = None;
            return None;
        };
        let since = match &self.candidate {
            Some((candidate, since)) if candidate.app_id == call.app_id => *since,
            _ => now_ms,
        };
        if now_ms - since < START_AFTER_MS {
            self.candidate = Some((call, since));
            return None;
        }
        self.candidate = None;
        self.active = Some(call.clone());
        Some(Change::Started(call))
    }

    /// Ends the call now, for example when audio is switched off.
    pub fn reset(&mut self) -> Option<Change> {
        self.candidate = None;
        self.quiet_since = None;
        self.active.take().map(|_| Change::Ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(pid: i32, bundle_id: &str) -> MicUser {
        MicUser {
            pid,
            bundle_id: bundle_id.into(),
            name: bundle_id.into(),
        }
    }

    #[test]
    fn a_known_call_app_using_the_mic_is_a_call() {
        let call = detect(&[user(10, "us.zoom.xos")], 1, &[]).expect("zoom");
        assert_eq!(call.app_id, "us.zoom.xos");
        let helper = detect(&[user(11, "com.google.Chrome.helper")], 1, &[]).expect("chrome");
        assert_eq!(helper.app_id, "com.google.Chrome");
    }

    #[test]
    fn a_call_lists_every_process_of_the_app_that_uses_the_mic() {
        let users = [
            user(10, "com.google.Chrome.helper"),
            user(11, "com.example.Other"),
            user(12, "com.google.Chrome.helper.Renderer"),
            user(1, "com.google.Chrome"),
        ];
        assert_eq!(detect(&users, 1, &[]).expect("chrome").pids, [10, 12]);
    }

    #[test]
    fn unknown_apps_and_lookalike_ids_are_not_calls() {
        assert!(detect(&[user(10, "com.example.Dictation")], 1, &[]).is_none());
        assert!(detect(&[user(10, "us.zoom.xos2")], 1, &[]).is_none());
        assert!(detect(&[], 1, &[]).is_none());
    }

    #[test]
    fn sextant_itself_never_counts_even_when_marked() {
        let marked = ["com.sextant.app".to_string()];
        assert!(detect(&[user(7, "com.sextant.app")], 7, &marked).is_none());
    }

    #[test]
    fn a_marked_app_is_a_call() {
        let marked = ["com.example.Dictation".to_string()];
        let call = detect(&[user(10, "com.example.Dictation")], 1, &marked).expect("marked");
        assert_eq!(call.app_id, "com.example.Dictation");
    }

    #[test]
    fn the_first_call_app_wins_when_several_use_the_mic() {
        let users = [user(10, "com.example.Other"), user(11, "us.zoom.xos")];
        assert_eq!(detect(&users, 1, &[]).expect("zoom").app_id, "us.zoom.xos");
    }

    fn zoom() -> Call {
        Call {
            app_id: "us.zoom.xos".into(),
            name: "zoom.us".into(),
            pids: vec![10],
        }
    }

    #[test]
    fn a_call_starts_after_two_seconds_and_ends_after_five_quiet_seconds() {
        let mut tracker = CallTracker::new();
        assert_eq!(tracker.step(0, Some(zoom())), None);
        assert_eq!(tracker.step(1_500, Some(zoom())), None);
        assert_eq!(
            tracker.step(2_000, Some(zoom())),
            Some(Change::Started(zoom()))
        );
        assert_eq!(tracker.step(3_000, None), None);
        assert_eq!(tracker.step(7_000, None), None);
        assert_eq!(tracker.step(8_000, None), Some(Change::Ended));
        assert!(tracker.active().is_none());
    }

    #[test]
    fn a_short_beep_or_a_mute_does_not_flap() {
        let mut tracker = CallTracker::new();
        assert_eq!(tracker.step(0, Some(zoom())), None);
        assert_eq!(tracker.step(500, None), None);
        assert_eq!(
            tracker.step(2_500, Some(zoom())),
            None,
            "the clock restarts"
        );
        assert_eq!(
            tracker.step(4_500, Some(zoom())),
            Some(Change::Started(zoom()))
        );
        assert_eq!(tracker.step(5_000, None), None);
        assert_eq!(tracker.step(6_000, Some(zoom())), None, "back before 5 s");
        assert_eq!(tracker.step(10_900, None), None);
        assert!(tracker.active().is_some());
    }

    #[test]
    fn reset_ends_an_active_call_at_once() {
        let mut tracker = CallTracker::new();
        tracker.step(0, Some(zoom()));
        tracker.step(2_000, Some(zoom()));
        assert_eq!(tracker.reset(), Some(Change::Ended));
        assert_eq!(tracker.reset(), None);
    }
}
