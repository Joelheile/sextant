//! Actions (routines-plan.md T1): presses and edits, each attached to the activity it happened in.

use super::entities::{ValueType, value_type};
use crate::model::Payload;
use crate::text::template;

/// One press or edit. The value is the stored one, already redacted at write time.
pub(crate) struct Act {
    pub symbol: String,
    pub value_type: Option<ValueType>,
    pub value: Option<String>,
}

/// App, role and label, with numbers and ids replaced the way window titles are.
pub(crate) fn symbol(payload: &Payload) -> String {
    format!(
        "{} · {} · {}",
        payload.app,
        payload.role,
        template(&payload.label)
    )
}

pub(crate) fn act(payload: &Payload) -> Act {
    let value = (!payload.value.is_empty()).then(|| payload.value.clone());
    Act {
        symbol: symbol(payload),
        value_type: value.as_deref().map(value_type),
        value,
    }
}

/// The value when it names one thing: an email, an id or an amount.
pub(crate) fn identifier(act: &Act) -> Option<&str> {
    match act.value_type {
        Some(ValueType::Email | ValueType::Id | ValueType::Amount) => act.value.as_deref(),
        _ => None,
    }
}

/// The index of the span each time falls in, or none. Spans are sorted by start and do not overlap.
pub(crate) fn attach(spans: &[(i64, i64)], times: &[i64]) -> Vec<Option<usize>> {
    times
        .iter()
        .map(|&time| {
            let started = spans.partition_point(|span| span.0 <= time);
            started
                .checked_sub(1)
                .filter(|&index| time < spans[index].1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{attach, symbol};
    use crate::model::Payload;

    fn payload(value: serde_json::Value) -> Payload {
        serde_json::from_value(value).expect("test setup")
    }

    #[test]
    fn a_symbol_names_the_app_role_and_label_with_numbers_replaced() {
        let send = payload(json!({"app": "Mail", "role": "AXButton", "label": "Send"}));
        assert_eq!(symbol(&send), "Mail · AXButton · send");
        let open = payload(json!({
            "app": "Google Chrome", "role": "AXButton", "label": "Open invoice 4711"
        }));
        assert_eq!(symbol(&open), "Google Chrome · AXButton · open invoice #");
    }

    #[test]
    fn an_action_belongs_to_the_activity_that_contains_its_time() {
        let spans = [(100, 200), (200, 300)];
        let times = [150, 200, 299, 301, 50];
        assert_eq!(
            attach(&spans, &times),
            [Some(0), Some(1), Some(1), None, None]
        );
    }
}
