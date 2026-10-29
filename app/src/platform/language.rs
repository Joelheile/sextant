//! Apple's on-device language model through `swift/Language.swift`. Every call blocks the calling
//! thread until the model answers, and returns `None` when Apple Intelligence is off, the model
//! refuses or the output does not parse. The caller then falls back to rules.

use std::ffi::{CStr, CString, c_char};

use serde::{Deserialize, Serialize};

unsafe extern "C" {
    fn sextant_language_plan(question: *const c_char) -> *mut c_char;
    fn sextant_language_answer(input: *const c_char) -> *mut c_char;
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

#[derive(Serialize)]
struct AnswerInput<'a> {
    question: &'a str,
    notes: &'a [Note],
}

/// Calls `function` with `input` and parses the JSON it returns.
fn call<T: for<'de> Deserialize<'de>>(
    function: unsafe extern "C" fn(*const c_char) -> *mut c_char,
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

/// Turns a question into search terms and a time span.
pub fn plan(question: &str) -> Option<Plan> {
    call(sextant_language_plan, question)
}

/// A short answer to `question` from `notes`, citing one of them.
pub fn answer(question: &str, notes: &[Note]) -> Option<Answer> {
    let input = serde_json::to_string(&AnswerInput { question, notes }).ok()?;
    call(sextant_language_answer, &input)
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
        let answer = answer("What did I tell Anna about pricing?", &notes).expect("answer");
        println!("{answer:?}");
        assert_eq!(answer.note, 2);
        assert!(answer.text.contains("40"));
    }
}
