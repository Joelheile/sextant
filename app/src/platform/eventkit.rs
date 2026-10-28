//! Calendar and Reminders through EventKit. Titles go straight into backfill events: this module
//! logs nothing.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore, EKReminder};
use objc2_foundation::{NSArray, NSDate, NSError};
use sextant_core::backfill::{CalendarRecord, ReminderRecord};
use sextant_core::{Error, Result};

const ANSWER_WAIT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Events,
    Reminders,
}

impl Kind {
    fn entity(self) -> EKEntityType {
        match self {
            Self::Events => EKEntityType::Event,
            Self::Reminders => EKEntityType::Reminder,
        }
    }
}

/// `granted`, `undetermined` (not asked yet) or `denied`.
pub fn status(kind: Kind) -> &'static str {
    // SAFETY: a class method that only reads the permission state.
    let status = unsafe { EKEventStore::authorizationStatusForEntityType(kind.entity()) };
    match status {
        EKAuthorizationStatus::FullAccess => "granted",
        EKAuthorizationStatus::NotDetermined => "undetermined",
        _ => "denied",
    }
}

/// Shows the system prompt and waits for the answer. Returns whether access was granted.
pub fn request(kind: Kind) -> Result<bool> {
    // SAFETY: plain Objective-C object creation.
    let store = unsafe { EKEventStore::new() };
    let (answer, answered) = mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = answer.send(granted.as_bool());
    });
    // SAFETY: the block stays alive until `answered` receives, and the store outlives the call.
    unsafe {
        match kind {
            Kind::Events => {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&handler))
            }
            Kind::Reminders => {
                store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&handler))
            }
        }
    }
    answered
        .recv_timeout(ANSWER_WAIT)
        .map_err(|_| Error::Platform("no answer to the permission prompt".into()))
}

fn require_access(kind: Kind, name: &str) -> Result<()> {
    match status(kind) {
        "granted" => Ok(()),
        _ => Err(Error::Platform(format!("{name} access is not granted"))),
    }
}

fn seconds(date: &NSDate) -> i64 {
    date.timeIntervalSince1970() as i64
}

pub fn events(from: i64, to: i64) -> Result<Vec<CalendarRecord>> {
    require_access(Kind::Events, "calendar")?;
    // SAFETY: the predicate comes from the same store, as EventKit requires.
    let found = unsafe {
        let store = EKEventStore::new();
        let predicate = store.predicateForEventsWithStartDate_endDate_calendars(
            &NSDate::dateWithTimeIntervalSince1970(from as f64),
            &NSDate::dateWithTimeIntervalSince1970(to as f64),
            None,
        );
        store.eventsMatchingPredicate(&predicate)
    };
    Ok(found
        .iter()
        .map(|event| {
            // SAFETY: read-only property getters on a live EKEvent.
            unsafe {
                CalendarRecord {
                    title: event.title().to_string(),
                    start: seconds(&event.startDate()),
                    end: seconds(&event.endDate()),
                    all_day: event.isAllDay(),
                }
            }
        })
        .collect())
}

pub fn reminders(from: i64, to: i64) -> Result<Vec<ReminderRecord>> {
    require_access(Kind::Reminders, "reminders")?;
    // SAFETY: the predicate comes from the same store. The block runs once on an EventKit queue.
    let fetched = unsafe {
        let store = EKEventStore::new();
        let predicate = store.predicateForRemindersInCalendars(None);
        let (done, fetched) = mpsc::channel();
        let handler = RcBlock::new(move |found: *mut NSArray<EKReminder>| {
            let records: Vec<ReminderRecord> = Retained::retain(found)
                .map(|found| found.iter().map(|reminder| record(&reminder)).collect())
                .unwrap_or_default();
            let _ = done.send(records);
        });
        store.fetchRemindersMatchingPredicate_completion(&predicate, &handler);
        fetched
            .recv_timeout(ANSWER_WAIT)
            .map_err(|_| Error::Platform("reminders did not load".into()))?
    };
    let in_range = |ts: Option<i64>| ts.is_some_and(|ts| ts >= from && ts <= to);
    Ok(fetched
        .into_iter()
        .filter(|r| in_range(r.created) || in_range(r.due))
        .collect())
}

fn record(reminder: &EKReminder) -> ReminderRecord {
    // SAFETY: read-only property getters on a live EKReminder.
    unsafe {
        ReminderRecord {
            title: reminder.title().to_string(),
            created: reminder.creationDate().map(|d| seconds(&d)),
            due: reminder
                .dueDateComponents()
                .and_then(|c| c.date())
                .map(|d| seconds(&d)),
            done: reminder.isCompleted(),
        }
    }
}
