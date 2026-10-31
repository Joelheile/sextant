//! What the menu bar shows about your day: active time, apps, switching, calls and how much time
//! went into workflows an agent could take. Computed from live episodes only, so backfill never
//! counts as time at the computer. Shown to the user on this Mac, never sent anywhere.

use std::collections::{HashMap, HashSet};

use rusqlite::params;

use crate::error::Result;
use crate::finder::Action;
use crate::store::Store;
use crate::time::{DAY, Local, WEEK_DAYS, day};
use crate::writer::unpack;

const TOP_APPS: usize = 5;
/// Episodes closer than this belong to one stretch of work.
const RUN_GAP_S: i64 = 120;

#[derive(Debug, Clone, PartialEq)]
pub struct AppTime {
    pub app: String,
    pub minutes: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Insights {
    /// Minutes at the computer today.
    pub active_minutes: f64,
    /// Active minutes of the last 7 days, oldest first, today last.
    pub days: Vec<f64>,
    /// The apps that took the most time today.
    pub apps: Vec<AppTime>,
    /// App changes within a stretch of work today.
    pub switches: usize,
    /// `None` until there is a minute of active time.
    pub switches_per_hour: Option<f64>,
    /// The longest stretch in one app today.
    pub longest_focus_minutes: f64,
    /// Distinct screen lines captured today.
    pub lines: usize,
    /// Minutes with call audio today. Overlapping speakers count once.
    pub call_minutes: f64,
    /// When the last screen was captured.
    pub last_capture: Option<i64>,
    /// Hours in the steps of workflows to automate or assist, the last 7 days.
    pub automatable_hours: f64,
    /// The same for the 7 days before.
    pub automatable_hours_last_week: f64,
}

struct Span {
    start: i64,
    end: i64,
    app: String,
    step: i64,
}

impl Store {
    pub fn insights(&self, when: Local) -> Result<Insights> {
        let Local { now, utc_offset_s } = when;
        let today = day(now, utc_offset_s) * DAY - i64::from(utc_offset_s);
        let spans = self.live_spans(now - 2 * WEEK_DAYS * DAY, now)?;
        let in_range = |from: i64, to: i64| {
            spans.iter().filter_map(move |s| {
                let (start, end) = (s.start.max(from), s.end.min(to));
                (end > start).then_some((s, end - start))
            })
        };
        let minutes =
            |from: i64, to: i64| in_range(from, to).map(|(_, s)| s).sum::<i64>() as f64 / 60.0;

        let days = (0..WEEK_DAYS)
            .rev()
            .map(|back| {
                let start = today - back * DAY;
                minutes(start, (start + DAY).min(now))
            })
            .collect();

        let mut by_app: HashMap<&str, i64> = HashMap::new();
        for (span, seconds) in in_range(today, now) {
            *by_app.entry(&span.app).or_default() += seconds;
        }
        let mut apps: Vec<AppTime> = by_app
            .into_iter()
            .map(|(app, seconds)| AppTime {
                app: app.to_owned(),
                minutes: seconds as f64 / 60.0,
            })
            .collect();
        apps.sort_by(|a, b| b.minutes.total_cmp(&a.minutes).then(a.app.cmp(&b.app)));
        apps.truncate(TOP_APPS);

        let today_spans: Vec<&Span> = spans.iter().filter(|s| s.end > today).collect();
        let (switches, longest_s) = focus(&today_spans);
        let active_minutes = minutes(today, now);

        let automatable = self.automatable_steps()?;
        let automatable_hours = |from: i64, to: i64| {
            in_range(from, to)
                .filter(|(s, _)| automatable.contains(&s.step))
                .map(|(_, seconds)| seconds)
                .sum::<i64>() as f64
                / 3600.0
        };

        Ok(Insights {
            active_minutes,
            days,
            apps,
            switches,
            switches_per_hour: (active_minutes >= 1.0)
                .then(|| switches as f64 / (active_minutes / 60.0)),
            longest_focus_minutes: longest_s as f64 / 60.0,
            lines: self.lines_since(today)?,
            call_minutes: self.call_seconds(today, now)? as f64 / 60.0,
            last_capture: self.last_capture()?.map(|(ts, _)| ts),
            automatable_hours: automatable_hours(now - WEEK_DAYS * DAY, now),
            automatable_hours_last_week: automatable_hours(
                now - 2 * WEEK_DAYS * DAY,
                now - WEEK_DAYS * DAY,
            ),
        })
    }

    fn live_spans(&self, from: i64, to: i64) -> Result<Vec<Span>> {
        let mut statement = self.conn.prepare(
            "SELECT start_ts, end_ts, app, step_id FROM episodes
             WHERE live = 1 AND end_ts > ?1 AND start_ts < ?2 ORDER BY start_ts",
        )?;
        let rows = statement.query_map(params![from, to], |row| {
            Ok(Span {
                start: row.get(0)?,
                end: row.get(1)?,
                app: row.get(2)?,
                step: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Step ids of the workflows the finder marks to automate or assist.
    fn automatable_steps(&self) -> Result<HashSet<i64>> {
        let keys: HashSet<String> = self
            .workflows()?
            .into_iter()
            .filter(|w| matches!(w.action, Action::Automate | Action::Assist))
            .flat_map(|w| w.steps.into_iter().map(|s| s.key))
            .collect();
        let mut statement = self.conn.prepare("SELECT id, key FROM steps")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut ids = HashSet::new();
        for row in rows {
            let (id, key) = row?;
            if keys.contains(&key) {
                ids.insert(id);
            }
        }
        Ok(ids)
    }

    /// Reads the line ids packed into each frame since `from`, so it seeks `frames_ts` instead of
    /// scanning all of `frame_lines`.
    fn lines_since(&self, from: i64) -> Result<usize> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT packed FROM frames WHERE ts >= ?1")?;
        let mut rows = statement.query(params![from])?;
        let mut lines = HashSet::new();
        while let Some(row) = rows.next()? {
            lines.extend(
                unpack(&row.get::<_, Vec<u8>>(0)?)
                    .into_iter()
                    .map(|(id, _)| id),
            );
        }
        Ok(lines.len())
    }

    /// When the last screen was captured and in which app.
    pub fn last_capture(&self) -> Result<Option<(i64, String)>> {
        let found = self.conn.query_row(
            "SELECT f.ts, w.app FROM frames f JOIN windows w ON w.id = f.window_id
             ORDER BY f.ts DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match found {
            Ok(last) => Ok(Some(last)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Seconds covered by at least one utterance between `from` and `to`.
    fn call_seconds(&self, from: i64, to: i64) -> Result<i64> {
        let mut statement = self.conn.prepare(
            "SELECT start_ts, end_ts FROM utterances
             WHERE end_ts > ?1 AND start_ts < ?2 ORDER BY start_ts",
        )?;
        let rows = statement.query_map(params![from, to], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut total = 0;
        let mut open: Option<(i64, i64)> = None;
        for row in rows {
            let (start, end) = row?;
            let (start, end) = (start.max(from), end.min(to));
            open = match open {
                Some((s, e)) if start <= e => Some((s, e.max(end))),
                Some((s, e)) => {
                    total += e - s;
                    Some((start, end))
                }
                None => Some((start, end)),
            };
        }
        Ok(total + open.map_or(0, |(s, e)| e - s))
    }
}

/// App switches within stretches of work, and the longest stretch in one app, in seconds.
fn focus(spans: &[&Span]) -> (usize, i64) {
    let mut switches = 0;
    let mut longest = 0;
    let mut run: Option<(&str, i64, i64)> = None;
    for span in spans {
        run = match run {
            Some((app, start, end)) if span.start - end <= RUN_GAP_S => {
                if app == span.app {
                    Some((app, start, span.end))
                } else {
                    switches += 1;
                    longest = longest.max(end - start);
                    Some((&span.app, span.start, span.end))
                }
            }
            Some((_, start, end)) => {
                longest = longest.max(end - start);
                Some((&span.app, span.start, span.end))
            }
            None => Some((&span.app, span.start, span.end)),
        };
    }
    if let Some((_, start, end)) = run {
        longest = longest.max(end - start);
    }
    (switches, longest)
}
