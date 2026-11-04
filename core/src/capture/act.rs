//! Actions: what the user did to which control, read from the Accessibility tree.
//! A press on a labelled control and the value left in a text field are the UI log that
//! the finder reads. Never keystrokes, never a secure field.

use serde_json::json;

use super::event;
use crate::model::{Event, EventKind};

/// Longer values are documents, which the screen text already holds.
pub const MAX_VALUE_CHARS: usize = 2000;
/// A press looks this far up from the element under the pointer for the control it hit.
pub const MAX_DEPTH: usize = 6;
/// Longer labels are content, not the name of a control.
const MAX_LABEL_CHARS: usize = 80;

const PRESSABLE: [&str; 11] = [
    "AXButton",
    "AXLink",
    "AXMenuItem",
    "AXMenuBarItem",
    "AXMenuButton",
    "AXPopUpButton",
    "AXCheckBox",
    "AXRadioButton",
    "AXDisclosureTriangle",
    "AXRow",
    "AXCell",
];

const EDITABLE: [&str; 3] = ["AXTextField", "AXTextArea", "AXComboBox"];

/// One element as the Accessibility tree describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Element {
    pub role: String,
    pub subrole: String,
    pub title: String,
    pub description: String,
    pub placeholder: String,
    /// Read only for static text, where it is the label.
    pub value: String,
}

impl Element {
    fn secure(&self) -> bool {
        self.role == "AXSecureTextField" || self.subrole == "AXSecureTextField"
    }

    /// Title, then description, then placeholder; static text names itself.
    fn label(&self) -> String {
        let own = [&self.title, &self.description, &self.placeholder];
        let text = own
            .into_iter()
            .chain((self.role == "AXStaticText").then_some(&self.value))
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
            .unwrap_or_default();
        text.chars().take(MAX_LABEL_CHARS).collect()
    }
}

pub fn pressable(element: &Element) -> bool {
    PRESSABLE.contains(&element.role.as_str())
}

/// A text field whose value may be read. Secure fields never are.
pub fn editable(element: &Element) -> bool {
    !element.secure() && EDITABLE.contains(&element.role.as_str())
}

/// `chain` runs from the element under the pointer up through its parents. The press goes to the
/// nearest pressable control, named by its own label or else by the text that was clicked.
pub fn press_event(ts: i64, app: &str, chain: &[Element]) -> Option<Event> {
    let chain = &chain[..chain.len().min(MAX_DEPTH)];
    if chain.iter().any(Element::secure) {
        return None;
    }
    let at = chain.iter().position(pressable)?;
    let control = &chain[at];
    let label = std::iter::once(control)
        .chain(&chain[..at])
        .map(Element::label)
        .find(|label| !label.is_empty())?;
    Some(event(
        ts,
        EventKind::Press,
        json!({"app": app, "role": control.role, "label": label}),
    ))
}

/// The focused element on one poll. `value` is `None` when it was too long to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Stays the same while focus stays on the same element.
    pub id: u64,
    pub element: Element,
    pub value: Option<String>,
}

struct Open {
    id: u64,
    app: String,
    role: String,
    label: String,
    /// The value when focus arrived or when it was last written.
    entry: Option<String>,
    last: Option<String>,
}

impl Open {
    /// An edit when the value differs from what was there before.
    fn settle(&mut self, ts: i64, value: Option<String>) -> Option<Event> {
        let value = value?;
        let text = value.trim();
        let changed = self.entry.as_deref().map(str::trim) != Some(text);
        let entry = self.entry.replace(value.clone());
        if !changed || text.is_empty() || entry.is_none() {
            return None;
        }
        Some(event(
            ts,
            EventKind::Edit,
            json!({"app": self.app, "role": self.role, "label": self.label, "value": text}),
        ))
    }
}

/// Follows the focused text field from poll to poll and says what was left in it.
#[derive(Default)]
pub struct Edits {
    open: Option<Open>,
}

impl Edits {
    pub fn new() -> Self {
        Self::default()
    }

    /// One poll. `focused` is `None` when focus left the app or capture may not read it.
    /// `reread` reads the field that just lost focus once more, so text typed right before Tab
    /// or a click is not lost.
    pub fn observe(
        &mut self,
        ts: i64,
        app: &str,
        focused: Option<Field>,
        reread: impl FnOnce(u64) -> Option<String>,
    ) -> Option<Event> {
        let focused = focused.filter(|field| editable(&field.element));
        if let (Some(open), Some(field)) = (&mut self.open, &focused)
            && open.id == field.id
        {
            let value = field.value.clone();
            // A message field empties when it is sent: keep what was in it.
            let sent = value.as_deref().is_some_and(|v| v.trim().is_empty())
                && open.last.as_deref().is_some_and(|v| !v.trim().is_empty());
            let edit = if sent {
                let last = open.last.clone();
                let edit = open.settle(ts, last);
                open.entry = value.clone();
                edit
            } else {
                None
            };
            open.last = value;
            return edit;
        }
        let left = self.open.take().and_then(|mut open| {
            let value = reread(open.id).or_else(|| open.last.clone());
            open.settle(ts, value)
        });
        self.open = focused.map(|field| Open {
            id: field.id,
            app: app.to_string(),
            role: field.element.role.clone(),
            label: field.element.label(),
            entry: field.value.clone(),
            last: field.value,
        });
        left
    }

    /// Writes what is in the focused field now, before a press that may act on it.
    pub fn checkpoint(
        &mut self,
        ts: i64,
        reread: impl FnOnce(u64) -> Option<String>,
    ) -> Option<Event> {
        let open = self.open.as_mut()?;
        let value = reread(open.id).or_else(|| open.last.clone());
        open.last = value.clone();
        open.settle(ts, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(role: &str, title: &str) -> Element {
        Element {
            role: role.into(),
            title: title.into(),
            ..Element::default()
        }
    }

    fn text(value: &str) -> Element {
        Element {
            role: "AXStaticText".into(),
            value: value.into(),
            ..Element::default()
        }
    }

    fn field(id: u64, label: &str, value: &str) -> Field {
        Field {
            id,
            element: Element {
                role: "AXTextField".into(),
                description: label.into(),
                ..Element::default()
            },
            value: Some(value.into()),
        }
    }

    fn json(event: &Option<Event>) -> serde_json::Value {
        event.as_ref().map(|e| e.json.clone()).unwrap_or_default()
    }

    #[test]
    fn a_press_names_the_control_or_the_text_inside_it() {
        let send = press_event(1, "Mail", &[element("AXButton", "Send")]);
        assert_eq!(
            json(&send),
            json!({"app": "Mail", "role": "AXButton", "label": "Send"})
        );
        assert_eq!(send.map(|e| e.kind), Some(EventKind::Press));

        let link = press_event(
            1,
            "Chrome",
            &[
                text("Open invoices"),
                element("AXLink", ""),
                element("AXWebArea", "Acme"),
            ],
        );
        assert_eq!(json(&link)["label"], "Open invoices");
        assert_eq!(json(&link)["role"], "AXLink");
    }

    #[test]
    fn presses_on_plain_text_unlabelled_controls_or_too_deep_are_dropped() {
        assert!(press_event(1, "Mail", &[text("Hello"), element("AXGroup", "")]).is_none());
        assert!(press_event(1, "Mail", &[element("AXButton", "")]).is_none());
        let mut deep = vec![text("Save"); MAX_DEPTH];
        deep.push(element("AXButton", "Save"));
        assert!(press_event(1, "Mail", &deep).is_none());
    }

    #[test]
    fn a_long_label_is_cut() {
        let long = "x".repeat(200);
        let event = press_event(1, "Mail", &[element("AXRow", ""), text(&long)]);
        assert!(event.is_none(), "text above the row is not its label");
        let event = press_event(1, "Mail", &[text(&long), element("AXRow", "")]);
        assert_eq!(
            json(&event)["label"].as_str().map(str::len),
            Some(MAX_LABEL_CHARS)
        );
    }

    #[test]
    fn leaving_a_changed_field_writes_its_value_with_the_label() {
        let mut edits = Edits::new();
        let none = |_| None;
        assert!(
            edits
                .observe(1, "Mail", Some(field(7, "Subject", "")), none)
                .is_none()
        );
        assert!(
            edits
                .observe(2, "Mail", Some(field(7, "Subject", "Invoice")), none)
                .is_none()
        );
        // Typed " 4711" and pressed Tab within one poll: the reread sees it.
        let left = edits.observe(3, "Mail", Some(field(8, "To", "")), |id| {
            (id == 7).then(|| "Invoice 4711 ".to_string())
        });
        assert_eq!(
            json(&left),
            json!({"app": "Mail", "role": "AXTextField", "label": "Subject", "value": "Invoice 4711"})
        );
        assert_eq!(left.map(|e| e.kind), Some(EventKind::Edit));
    }

    #[test]
    fn an_unchanged_or_emptied_field_writes_nothing() {
        let mut edits = Edits::new();
        let none = |_| None;
        edits.observe(1, "Mail", Some(field(7, "Subject", "Re: Invoice")), none);
        assert!(
            edits
                .observe(2, "Mail", Some(field(8, "To", "")), none)
                .is_none()
        );
        edits.observe(3, "Mail", Some(field(8, "To", "")), none);
        assert!(edits.observe(4, "Mail", None, none).is_none());
    }

    #[test]
    fn a_message_field_that_empties_on_send_keeps_what_was_typed() {
        let mut edits = Edits::new();
        let none = |_| None;
        edits.observe(1, "Slack", Some(field(3, "Message #general", "")), none);
        edits.observe(
            2,
            "Slack",
            Some(field(3, "Message #general", "Invoice sent")),
            none,
        );
        let sent = edits.observe(3, "Slack", Some(field(3, "Message #general", "")), none);
        assert_eq!(json(&sent)["value"], "Invoice sent");
        assert!(
            edits.observe(4, "Slack", None, none).is_none(),
            "written once"
        );
    }

    #[test]
    fn a_checkpoint_writes_before_a_press_and_not_again_on_leaving() {
        let mut edits = Edits::new();
        let none = |_| None;
        edits.observe(1, "Mail", Some(field(7, "Subject", "")), none);
        let before = edits.checkpoint(2, |_| Some("Invoice 4711".into()));
        assert_eq!(json(&before)["value"], "Invoice 4711");
        assert!(edits.observe(3, "Mail", None, none).is_none());
    }

    #[test]
    fn secure_long_and_non_text_fields_are_never_written() {
        let mut edits = Edits::new();
        let none = |_| None;
        let mut secure = field(1, "Password", "");
        secure.element.subrole = "AXSecureTextField".into();
        edits.observe(1, "Mail", Some(secure.clone()), none);
        secure.value = Some("hunter2".into());
        edits.observe(2, "Mail", Some(secure), none);
        assert!(
            edits
                .observe(3, "Mail", None, |_| Some("hunter2".into()))
                .is_none()
        );

        let mut long = field(2, "Body", "");
        long.value = None;
        edits.observe(4, "Mail", Some(long.clone()), none);
        assert!(edits.observe(5, "Mail", None, none).is_none());

        let mut button = field(3, "Send", "");
        button.element.role = "AXButton".into();
        edits.observe(6, "Mail", Some(button), none);
        assert!(
            edits
                .observe(7, "Mail", None, |_| Some("x".into()))
                .is_none()
        );
    }
}
