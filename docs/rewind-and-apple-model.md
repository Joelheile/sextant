# Rewind panel and Apple's on-device model (deprecated)

Deprecated on 2026-11-02 and removed from the code to make Sextant CLI first. This page records how they worked, so they can come back. The last commit with both is `904ade0`.

## What the user saw

Cmd-Shift-Space opened a translucent bar at the bottom centre of the screen. You typed a question such as "what did I write to Anna about pricing", and answers stacked upward like a chat. The panel showed up to 5 matching sessions at once, then a short answer from Apple's model that cited one of them. Past questions came back from the database.

## How it worked

1. **Plan.** Apple's model turned the question into search terms and a time span: any time, today, yesterday, this week, last week or this month. Without Apple Intelligence, the question's keywords were used over all time, with English and German filler words dropped (`core/src/recall.rs`, `keywords`).
2. **Recall.** Each term was searched on its own in the full-text index, at most 8 terms and 200 newest hits per term. Hits were grouped into sessions (same screen or speaker within 10 minutes), and sessions were ranked by how many terms they covered, counting the window title. So "Anna" in the title and "pricing" in the body found the email. If the time span found nothing, the search ran again over all time, because people often misremember when.
3. **Answer.** The first 6 lines of each session, at most 200 characters each, went to the model as numbered notes with app, title and time. The model streamed a short answer and the number of the note it used, which moved to the top. Instructions: answer only from the notes, never guess or add facts.
4. **History.** Each question and answer was redacted and saved in the `turns` table. The table stays in the schema; nothing writes to it now.

## Apple's model (L1)

- Foundation Models on macOS 26 with Apple Intelligence, M1 or later. Free, offline, 4,096-token context.
- `app/swift/Language.swift` was compiled by `app/build.rs` into a static library and called over C from `app/src/platform/language.rs`. Typed output came from a `GenerationSchema`. The plan session was prewarmed so the first question answered faster.
- Every call blocked its thread and returned nothing when Apple Intelligence was off, the model refused or the output did not parse. The caller then fell back to rules.
- It was only ever used by the rewind panel. The labels for tasks that `docs/routines-plan.md` L1 planned never used it.

## Why it went

- Your agent already answers the same questions through the MCP `search`, `context_at` and `timeline` tools. Those tools stay.
- The panel was a second surface next to the window and the popover, with its own hotkey, window code and Swift build step.

## Bringing it back

- As a CLI command: `sextant ask "<question>"` running the recall in step 2 and printing the sessions. That needs no model.
- With a model: Apple's model as above, or the user's own agent through the bridge.
- Recover the code with `git show 904ade0:<path>` for `app/src/rewind.rs`, the rewind half of `app/src/screens.rs`, `app/src/platform/language.rs`, `app/swift/Language.swift`, `app/build.rs`, `core/src/recall.rs`, `core/src/rewind.rs`, `core/tests/recall.rs`, `core/tests/rewind.rs` and `ui/Rewind.tsx`.
