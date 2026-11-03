//! §7.1 Activities: consecutive focus on one object key.

use crate::model::Payload;
use crate::text::{split_url, template, tokens};

const MIN_ACTIVITY_S: i64 = 5;
const MAX_ACTIVITY_S: i64 = 120 * 60;
const MERGE_GAP_S: i64 = 120;

pub(crate) struct EventRow {
    pub ts: i64,
    pub kind: String,
    pub live: bool,
    pub payload: Payload,
}

pub(crate) struct Activity {
    pub start: i64,
    pub end: i64,
    pub app: String,
    /// Host for web pages, else the app name.
    pub scope: String,
    pub key: String,
    pub tokens: Vec<String>,
    pub doc: String,
    /// Title, URL path and heading, for entity extraction.
    pub text: String,
    pub live: bool,
    pub step: usize,
}

struct Segment<'a> {
    start: i64,
    end: i64,
    app: &'a str,
    meeting: bool,
    payload: &'a Payload,
    live: bool,
}

fn close<'a>(open: &mut Option<Segment<'a>>, segments: &mut Vec<Segment<'a>>, at: i64) {
    if let Some(mut segment) = open.take() {
        segment.end = at.min(segment.start + MAX_ACTIVITY_S);
        segments.push(segment);
    }
}

/// Activities in time order. A focus event lasts until the next focus, idle or pause event.
pub(crate) fn build(events: &[EventRow], now: i64) -> Vec<Activity> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut open: Option<Segment> = None;
    for event in events {
        let payload = &event.payload;
        match event.kind.as_str() {
            "focus" => {
                close(&mut open, &mut segments, event.ts);
                open = Some(Segment {
                    start: event.ts,
                    end: event.ts,
                    app: &payload.app,
                    meeting: false,
                    payload,
                    live: event.live,
                });
            }
            "idle" | "pause" => close(&mut open, &mut segments, event.ts),
            "visit" => {
                let app = if payload.app.is_empty() {
                    "Browser"
                } else {
                    &payload.app
                };
                let end = event.ts + payload.dur_s;
                segments.push(Segment {
                    start: event.ts,
                    end,
                    app,
                    meeting: false,
                    payload,
                    live: event.live,
                });
            }
            "calendar" => {
                segments.push(Segment {
                    start: event.ts,
                    end: payload.end_ts.min(now),
                    app: "Calendar",
                    meeting: true,
                    payload,
                    live: event.live,
                });
            }
            _ => {}
        }
    }
    close(&mut open, &mut segments, now);
    segments.retain(|s| s.end - s.start >= MIN_ACTIVITY_S);
    segments.sort_by_key(|s| s.start);

    let mut activities: Vec<Activity> = Vec::new();
    for segment in segments {
        let (scope, key, tokens) = object_key(&segment);
        let text = format!(
            "{}\n{}\n{}",
            segment.payload.title, segment.payload.url, segment.payload.text
        );
        if let Some(last) = activities.last_mut() {
            let end = last.end.max(segment.end);
            if last.key == key
                && segment.start - last.end < MERGE_GAP_S
                && end - last.start <= MAX_ACTIVITY_S
            {
                last.end = end;
                last.live |= segment.live;
                last.text.push('\n');
                last.text.push_str(&text);
                continue;
            }
        }
        let doc = segment.payload.doc.clone();
        activities.push(Activity {
            start: segment.start,
            end: segment.end,
            app: segment.app.to_string(),
            scope,
            key,
            tokens,
            doc,
            text,
            live: segment.live,
            step: 0,
        });
    }
    activities
}

/// Returns scope, key and key tokens.
/// Web page: host + first 2 path segments + 3 title tokens. Else app + document.
/// Else app + title template, refined by the OCR heading when there is one.
fn object_key(segment: &Segment) -> (String, String, Vec<String>) {
    let payload = segment.payload;
    if segment.meeting {
        let key = format!("meeting|{}", template(&payload.title));
        return ("meeting".into(), key, tokens(&payload.title));
    }
    let (host, path) = split_url(&payload.url);
    if !host.is_empty() {
        let path: Vec<String> = path.iter().take(2).map(|part| path_part(part)).collect();
        let title: Vec<String> = tokens(&payload.title).into_iter().take(3).collect();
        let key = format!("{host}/{}|{}", path.join("/"), title.join(" "));
        let mut key_tokens: Vec<String> = path.iter().flat_map(|p| tokens(p)).collect();
        key_tokens.extend(title);
        return (host, key, key_tokens);
    }
    let app = segment.app.to_string();
    if !payload.doc.is_empty() {
        return (
            app.clone(),
            format!("{app}|{}", payload.doc),
            tokens(&payload.doc),
        );
    }
    let mut key = format!("{app}|{}", template(&payload.title));
    let mut key_tokens = tokens(&payload.title);
    if !payload.text.is_empty() {
        key = format!("{key}|{}", template(&payload.text));
        key_tokens.extend(tokens(&payload.text));
    }
    (app, key, key_tokens)
}

fn path_part(part: &str) -> String {
    if part.chars().any(|c| c.is_ascii_digit()) || part.len() >= 16 {
        "#".into()
    } else {
        part.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(ts: i64, app: &str, title: &str, url: &str) -> EventRow {
        EventRow {
            ts,
            kind: "focus".into(),
            live: true,
            payload: Payload {
                app: app.into(),
                title: title.into(),
                url: url.into(),
                ..Payload::default()
            },
        }
    }

    fn idle(ts: i64) -> EventRow {
        EventRow {
            ts,
            kind: "idle".into(),
            live: true,
            payload: Payload::default(),
        }
    }

    #[test]
    fn short_flickers_drop_and_neighbours_merge() {
        let events = [
            focus(0, "Mail", "Invoice INV-2026-0001", ""),
            focus(60, "Slack", "Slack", ""),
            focus(62, "Mail", "Invoice INV-2026-0002", ""),
            idle(100),
        ];
        let activities = build(&events, 1000);
        assert_eq!(
            activities.len(),
            1,
            "the 2 s Slack flicker is dropped, Mail merges"
        );
        assert_eq!((activities[0].start, activities[0].end), (0, 100));
        assert_eq!(activities[0].key, "Mail|invoice inv #");
    }

    #[test]
    fn activities_are_capped_at_two_hours() {
        let activities = build(&[focus(0, "Code", "main.rs", "")], 10 * 3600);
        assert_eq!(activities[0].end - activities[0].start, 7200);
    }

    #[test]
    fn web_keys_use_host_path_and_title_tokens() {
        let events = [
            focus(
                0,
                "Chrome",
                "Rechnung bearbeiten - lexoffice",
                "https://app.lexoffice.de/vouchers/INV-2026-0412/edit?x=1",
            ),
            idle(30),
        ];
        let activities = build(&events, 100);
        assert_eq!(
            activities[0].key,
            "app.lexoffice.de/vouchers/#|rechnung bearbeiten lexoffice"
        );
        assert_eq!(activities[0].scope, "app.lexoffice.de");
    }
}
