//! Apple's on-device language model through `swift/Language.swift`. Every call blocks the calling
//! thread until the model answers, and returns `None` when Apple Intelligence is off, the model
//! refuses or the output does not parse. The caller then falls back to rules.

use std::ffi::{CStr, CString, c_char, c_void};

use serde::{Deserialize, Serialize};

/// Receives the answer text so far; `context` is the `TextSink` passed to the call.
type Forward = unsafe extern "C" fn(context: *mut c_void, text: *const c_char);
type TextSink<'a> = &'a mut (dyn FnMut(&str) + Send);

unsafe extern "C" {
    fn sextant_language_prewarm();
    fn sextant_language_plan(question: *const c_char) -> *mut c_char;
    fn sextant_language_answer(
        input: *const c_char,
        forward: Forward,
        context: *mut c_void,
    ) -> *mut c_char;
    fn sextant_language_judge(input: *const c_char) -> *mut c_char;
    fn sextant_language_free(pointer: *mut c_char);
}

#[derive(Debug, Deserialize)]
pub struct Plan {
    pub terms: Vec<String>,
    /// "any time", "today", "yesterday", "this week", "last week" or "this month".
    pub time: String,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub app: String,
    pub title: String,
    pub ts: i64,
    pub lines: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Answer {
    pub text: String,
    /// Counts from 1; 0 when no note supports the answer.
    pub note: usize,
}

/// What the model sees of a recurring step: no screen text, only templated keys.
#[derive(Debug, Serialize)]
pub struct Step<'a> {
    pub name: &'a str,
    pub category: &'a str,
    pub apps: &'a [String],
    pub objects: &'a [String],
}

#[derive(Debug, Deserialize)]
struct Judgment {
    share: String,
    reason: String,
}

#[derive(Serialize)]
struct AnswerInput<'a> {
    question: &'a str,
    notes: &'a [Note],
}

/// Calls `function` with `input` and parses the JSON it returns.
fn call<T: for<'de> Deserialize<'de>>(
    function: impl FnOnce(*const c_char) -> *mut c_char,
    input: &str,
) -> Option<T> {
    let input = CString::new(input).ok()?;
    // SAFETY: `input` is a valid C string that outlives the call. The result is NULL or a
    // C string from `strdup`, which only `sextant_language_free` releases.
    unsafe {
        let output = function(input.as_ptr());
        if output.is_null() {
            return None;
        }
        let parsed = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).ok();
        sextant_language_free(output);
        parsed
    }
}

/// Loads the model in the background, so the next [`plan`] starts warm. Returns at once.
pub fn prewarm() {
    // SAFETY: takes no arguments and only starts loading the model.
    unsafe { sextant_language_prewarm() }
}

/// Turns a question into search terms and a time span.
pub fn plan(question: &str) -> Option<Plan> {
    // SAFETY: `call` passes a valid C string.
    call(|input| unsafe { sextant_language_plan(input) }, question)
}

/// A short answer to `question` from `notes`, citing one of them. `on_text` gets the answer text
/// so far each time the model adds to it.
pub fn answer(
    question: &str,
    notes: &[Note],
    mut on_text: impl FnMut(&str) + Send,
) -> Option<Answer> {
    let input = serde_json::to_string(&AnswerInput { question, notes }).ok()?;
    let mut sink: TextSink = &mut on_text;
    let context = (&raw mut sink).cast::<c_void>();
    // SAFETY: `call` passes a valid C string. `context` points at `sink`, which lives until the
    // call returns, and the model calls `forward` only before then.
    call(
        |input| unsafe { sextant_language_answer(input, forward, context) },
        &input,
    )
}

/// Hands streamed text from Swift to the `TextSink` behind `context`.
unsafe extern "C" fn forward(context: *mut c_void, text: *const c_char) {
    // SAFETY: `context` is the `TextSink` from `answer`, and `text` a C string Swift keeps alive
    // for this call.
    let (sink, text) = unsafe { (&mut *context.cast::<TextSink>(), CStr::from_ptr(text)) };
    if let Ok(text) = text.to_str() {
        sink(text);
    }
}

/// The share of `step` an agent could do, 0.1 to 0.9, and why.
pub fn judge(step: &Step) -> Option<(f64, String)> {
    let input = serde_json::to_string(step).ok()?;
    // SAFETY: `call` passes a valid C string.
    let judgment: Judgment = call(|input| unsafe { sextant_language_judge(input) }, &input)?;
    let share = match judgment.share.as_str() {
        "not at all" => 0.1,
        "a little" => 0.3,
        "half" => 0.5,
        "most" => 0.7,
        "nearly all" => 0.9,
        _ => return None,
    };
    Some((share, format!("{}: {}", judgment.share, judgment.reason)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs Apple Intelligence on, so it runs only on request:
    /// `cargo test -p sextant-app language -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_model_plans_and_answers_from_fixture_notes() {
        let plan = plan("What was I writing to Anna yesterday about the pricing?").expect("plan");
        println!("{plan:?}");
        assert!(
            plan.terms
                .iter()
                .any(|term| term.eq_ignore_ascii_case("anna"))
        );
        assert_eq!(plan.time, "yesterday");

        let notes = [
            Note {
                app: "Safari".into(),
                title: "Pricing pages".into(),
                ts: 1_800_000_000,
                lines: vec!["pricing for teams".into()],
            },
            Note {
                app: "Mail".into(),
                title: "Re: Offer - Anna Berg".into(),
                ts: 1_800_000_100,
                lines: vec![
                    "Hi Anna,".into(),
                    "the new pricing starts at 40 per seat".into(),
                ],
            },
        ];
        let mut streamed = String::new();
        let answer = answer("What did I tell Anna about pricing?", &notes, |text| {
            streamed = text.to_string();
        })
        .expect("answer");
        assert_eq!(
            streamed, answer.text,
            "the last streamed text is the answer"
        );
        println!("{answer:?}");
        assert_eq!(answer.note, 2);
        assert!(answer.text.contains("40"));

        let apps = ["Mail".to_string(), "Numbers".to_string()];
        let objects = ["invoice <n>.pdf".to_string(), "Re: Invoice <n>".to_string()];
        let judged = judge(&Step {
            name: "Copy invoice totals into a sheet",
            category: "spreadsheet",
            apps: &apps,
            objects: &objects,
        })
        .expect("judge");
        println!("{judged:?}");
        assert!(judged.0 >= 0.5, "copying is mostly automatable");
    }
}
