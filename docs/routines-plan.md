# Task graph and LLM layer plan

Status: T1 and T2 are built. T3 is deferred: nothing reads task types yet, and `from_tasks` in `core/src/finder/graph.rs` is the one grouping engine for now, so the MinHash and alignment code is deleted. T4 onward is not started. The plan builds on actions v1 (`press` and `edit` events, plan.md §4).

## The goal

Every knowledge worker should get two answers without setting anything up:

1. **Where does my time go?** By task ("invoice for Acme", "weekly report"), not by app.
2. **What could an agent take over, and how much time would that give back?**

## What exists today

| Tool                                                                                                                                                                                        | Input                                                       | How it finds work                                                                                                                                                               | Cost per user                                         | What it misses                                                                                           |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| [Screenpipe](https://github.com/screenpipe/screenpipe)                                                                                                                                      | Accessibility text first, OCR fallback, audio, input events | "Pipes": scheduled LLM agents that query the raw data. Team "repeated-action reports" ([task mining page](https://screenpipe.com/task-mining)) that don't explain their method. | Your LLM tokens over raw text                         | No published mining method. The LLM reads raw text each run.                                             |
| [Dayflow](https://github.com/JerryZLiu/Dayflow)                                                                                                                                             | Screenshots at 1 fps                                        | A vision model summarizes each 15-minute batch into a timeline                                                                                                                  | 2 Gemini calls or about 32 local calls per 15 minutes | A timeline only: no recurrence, no automation                                                            |
| [OpenAI Record & Replay](https://learn.chatgpt.com/docs/extend/record-and-replay), Claude ["Record a skill"](https://www.androidauthority.com/claude-cowork-record-skills-feature-3689919/) | One recording you start                                     | The model writes a `SKILL.md` from one demo                                                                                                                                     | One analysis per recording                            | You must already know what to automate. One run can't tell inputs that change each time from fixed ones. |
| Enterprise task mining: UiPath, Mimica, Skan, KYP.ai, Soroco, Celonis ([overview](https://kyp.ai/task-mining-tools-compared/))                                                              | Desktop agent, screenshots, computer vision, mostly Windows | ML clusters actions into process maps and automation reports                                                                                                                    | Quote-only, reportedly $100k+ a year                  | Sold to companies about their employees, not to the person doing the work                                |
| [TMI](https://arxiv.org/abs/2608.20319), research, 2026                                                                                                                                     | Screenshots and input events                                | A vision model grounds every event, then an LLM segments activities, assigns them to latent tasks and builds a hierarchical task model                                          | One frontier-model call per input event               | Best published quality, far too costly to run all day                                                    |
| [Work2Skill](https://arxiv.org/abs/2610.12984), research, 2026                                                                                                                              | One demo recording                                          | An agent inspects the evidence and writes a skill                                                                                                                               | 246k to 335k tokens per task                          | Per demo, like Record & Replay                                                                           |
| [Workflow induction toolkit](https://arxiv.org/abs/2510.22780), 2025                                                                                                                        | Computer-use trace                                          | An LLM writes phase-based workflow summaries                                                                                                                                    | LLM over the whole trace                              | Flat steps, assumes one task per trace                                                                   |

### What the research shows

- TMI has the best published results on finding tasks in passive recordings:
  - It recovers interleaved tasks with 0.974 agreement against ground truth.
  - It describes 74.9 % of steps correctly, against 30.3 % for the workflow-summary baseline.
  - Skills built from its task models raise held-out accuracy by 30 %.
- Three of its ideas carry over to Sextant:
  - Identifier anchors keep a task together across apps, because the same artifacts and names recur.
  - Each task gets two reconciled views: a goal hierarchy (why) and a control-flow tree (how: sequence, for-each, while).
  - Every node ties to an observed activity.
- TMI found error correction in 89 % of human sessions and exploratory search in 87 %, which a flat step list can't represent.
- Most of TMI's cost is grounding. HumanWork sessions average 1,300 raw events per 68 minutes, and each one goes to a frontier model with two screenshots. That is millions of tokens per hour of work, by my estimate.
- OpenAI and Anthropic both ship skills from recordings. Both write `SKILL.md` and both start after the user has decided what to automate, from a single run.

## Why Sextant can be cheaper and better

Sextant already records semantic actions: "pressed button Save in lexoffice" and "left 'Invoice 4711' in field Subject". Those come from the Accessibility tree, which TMI and Sharingan pay a vision model to reconstruct from pixels. So:

|                                    | TMI-style pipeline                 | Sextant                                                                   |
| ---------------------------------- | ---------------------------------- | ------------------------------------------------------------------------- |
| Grounding (what did this event do) | One vision-model call per event    | Free: the Accessibility tree names the control and field                  |
| Activities and tasks               | LLM per activity                   | Rules and identifier anchors. An LLM only for leftovers.                  |
| Recurring work                     | Not covered, one session at a time | Clustering across weeks of runs                                           |
| Inputs vs constants                | Guessed from one run               | Measured across runs: fixed, transferred from an earlier step, or varying |
| LLM calls                          | Per event                          | Once per recurring task type, on text of about 2k tokens                  |
| Updates                            | Batch over the whole trace         | Incremental, only new events                                              |

By my estimate, this is three orders of magnitude cheaper than per-event grounding, and it can run on the Mac. Sextant also sees the same work 10 or 50 times, so it can measure which values change, where they come from and how long the work takes. A single recording shows none of that.

## Graph layer (core, deterministic, `now` passed in)

### T1. Activities

An activity is one focus span on one object (the existing episode) plus the actions inside it.

- Add `'press'` and `'edit'` to `MAP_KINDS` (`core/src/finder/mod.rs`) and attach each action to its episode.
- **Action symbol:** `app · role · label template`, with numbers and ids replaced the same way window titles are (`steps.rs`).
- **Value type** for each edit: email hash, amount, date, id, number, short text or long text, from the rules in `entities.rs`.
- **Identifiers:** the entities already found (invoice and ticket ids, email hashes, document names, amounts), plus edit values of type id, email or amount.

### T2. Tasks (where time goes)

This is TMI's latent task induction, done with rules first.

1. Process activities in time order. Each open task keeps a profile: identifiers, apps and objects, and the last time it was seen.
2. An activity joins the task it shares the most identifiers with, among tasks seen in the last 7 days. Ties go to the most recent task.
3. With no shared identifier, it joins the task active within 2 minutes in the same object, or else stays unassigned.
4. A nightly pass merges tasks that share an identifier and splits tasks idle for over 14 days.
5. **LLM only for leftovers:** unassigned activities over 5 minutes go in one batch per day to the on-device model (L1), which names the task or says "none". The batch holds window title templates and app names, never screen text.

- This replaces cases (§7.5), which are the same idea with one identifier.
- Output: `tasks(id, label, workflow_id, first_ts, last_ts, active_s)` and `task_episodes(task_id, episode_id)`.
- "Where time goes" becomes task minutes per day, with apps underneath.

### T3. Task types (recurring work)

1. **Signature** of a task: the shingles of its action symbols (3 in a row) plus its step categories.
2. **Clustering:** MinHash with 64 hashes and locality-sensitive hashing (LSH) with Jaccard ≥ 0.5. Clustering is near-linear in the number of tasks and needs no new dependency, about 100 lines in core. It replaces the edit-distance grouping of workflows (§7.6).
3. A task type needs 3 or more tasks on 2 or more days.
4. **Procedure:** align each task's action sequence against the type's medoid, the task closest to all others, with the existing `edit_distance` extended to keep the alignment.
   - Actions present in 60 % or more of runs are the steps.
   - A block repeated within a run on different identifiers is a for-each.
   - A block repeated until the run ends is a while.
   - Error corrections and detours fall out as low-frequency actions and are kept as "sometimes".

### T4. Inputs and automatability

For each step of a task type, compare its value across runs:

| Class       | Rule                                                                                   | Agent                      |
| ----------- | -------------------------------------------------------------------------------------- | -------------------------- |
| fixed       | a press, or the same value in 80 % or more of runs                                     | yes                        |
| transferred | the value was visible earlier in the same task in another app (copied, or typed again) | yes                        |
| derived     | it varies, has no source, and is an amount, date or id                                 | likely: a rule exists      |
| judgment    | varying text with no source                                                            | needs an LLM or the person |

- "Visible earlier" searches `lines_fts` and earlier edits of the same task, matching on the redacted form.
- automatable = (fixed + transferred) / steps. It replaces the category prior in the potential score whenever a task type exists.
- Transfers let the Workflows screen name friction exactly, for example "you typed the invoice amount from Mail into lexoffice 23 times this month".

### T5. Incremental refresh

- Today each refresh recomputes 90 days, with a budget under 2 s.
- New design: T1 and T2 run only on events since the last refresh. MinHash signatures are stored per task, so T3 updates only touched clusters. T4 reruns only for changed types.
- Target: a refresh under 50 ms for 10 minutes of new events. The full rebuild stays as the nightly pass and as the test oracle: incremental results must equal a full rebuild on fixtures.

### T6. Storage

T2 shipped in `core/src/schema_v6.sql`: `tasks` and `task_episodes`. The tables below arrive with T3.

- `task_types(id, label, signature BLOB, runs, days, minutes_week, automatable)`
- `type_steps(type_id, idx, symbol, share, input_class, source_step)`
- No values in the new tables, only symbols, classes and positions. Values stay in `events` under retention.

### Tests and budgets

Fixtures:

- The invoice week with actions, interleaved with a second task (weekly report) and noise: a skipped field, a correction, an unrelated chat.
- 90 days generated at 700 actions a day, 63,000 in total.

Done when:

- The invoice and report tasks separate, with a pair-agreement score of 0.9 or more, the measure TMI uses.
- The invoice type has 3 transferred inputs and 1 judgment input.
- Incremental equals full rebuild.
- An incremental refresh takes under 50 ms; the full 90-day rebuild stays under 2 s.

## LLM layer

Rule: the LLM judges, code computes (CLAUDE.md, code rule 9). It sees one compact text per task type, never raw events.

### L1. On-device model for small judgments

- Apple's Foundation Models framework runs a local model on macOS 26 with Apple Intelligence on M1 or later. Typed output comes from `@Generable`, and the context is 4,096 tokens ([guide](https://www.atelier-socle.com/en/articles/foundation-models-api-guide)).
- It is free, offline and fits Sextant's rule that no data leaves the Mac.
- Use it for:
  - labels for tasks and task types
  - leftover activity assignment (T2.5)
  - template, rule or decision for "derived" and "judgment" inputs
- The framework is Swift-only. It needs a small Swift helper built into the app and called over a C interface. Without Apple Intelligence, these labels fall back to rules: shared title tokens, as today.

### L2. Claude and ChatGPT for skills and advice

| Client                     | Path                                                                                                                            |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Claude Desktop             | Local MCP server, shipped as a one-click `.mcpb` bundle ([Anthropic](https://www.anthropic.com/engineering/desktop-extensions)) |
| Claude Code                | `claude mcp add sextant -- <path>/sextant mcp` (exists)                                                                         |
| ChatGPT desktop, Codex CLI | Local MCP server through `~/.codex/config.toml` ([OpenAI](https://learn.chatgpt.com/docs/extend/mcp)); add "Connect to ChatGPT" |
| ChatGPT web                | Remote MCP only. Not supported: it would need a public endpoint into your history.                                              |

- New MCP tools:
  - `where_time_goes(from, to)`: task minutes per day
  - `list_task_types`
  - `get_task_type(id)`: steps, input classes, sources, minutes
- **Skill writing:** the agent bridge (`core/src/agent.rs`) sends one task type to your `claude -p` or `codex exec` and gets back a `SKILL.md`. That is the same format Record & Replay and Record a skill produce, so a skill Sextant finds plugs into either.
- **Grounding check:** every step in the answer cites a step index of the task type. Code rejects answers with unknown or missing citations, because models are known to invent steps ([WONDERBREAD](https://arxiv.org/abs/2406.13264)).
- **What models see:**
  - Sent: app, control label templates, input classes, value types, sources, minutes.
  - Never sent: values, screen text, transcripts or entities, unless you turn on "Show example values" for one task type.

### L3. Evaluation

- **Labels from the person:** the Workflows screen asks "Is this one task?" and "Is this name right?". Answers are corrections (§7.12) and also labels for measuring T2 and T3 on real data.
- **Fixtures:** 10 task types with hand-written skills. An ignored test runs the bridge and scores step precision and recall.

## Order

| Step | Delivers                                                           | Done when                                                    |
| ---- | ------------------------------------------------------------------ | ------------------------------------------------------------ |
| T1   | Activities with actions and identifiers                            | The fixture shows actions per episode                        |
| T2   | Tasks, "where time goes"                                           | Pair agreement ≥ 0.9 on the interleaved fixture              |
| T3   | Task types and procedures (deferred)                               | The invoice type found with its steps and for-each           |
| T4   | Input classes, automatability in scores                            | Snapshots updated, transfers found                           |
| T5   | Incremental refresh                                                | Equals full rebuild, under 50 ms                             |
| L2   | MCP tools, Connect to ChatGPT, `.mcpb`, `SKILL.md` from task types | **(user)** Claude Desktop and ChatGPT desktop list the tools |
| L1   | On-device labels                                                   | Labels on the fixture without network                        |

## Open decisions

1. **Swift helper for Apple's on-device model (L1):** decided, and shipped. `app/swift/Language.swift` is compiled by `app/build.rs` and called from `app/src/platform/language.rs`. Without Apple Intelligence it returns nothing and the caller falls back to rules.
2. **Keyboard monitor** for Enter, Tab, Escape and modifier shortcuts only. Without it, "send with Enter" and shortcut-only steps are missing from procedures. It changes the "never keyboard hooks" rule in §4 and needs Input Monitoring permission.
3. **"Show example values" per task type**, or values never leave the Mac in any form.
