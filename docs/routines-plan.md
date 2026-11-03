# Task graph and LLM layer plan

Status: T1 and T2 are built. T3 is deferred: nothing reads task types yet, and `from_tasks` in `core/src/finder/graph.rs` is the one grouping engine for now, so the MinHash and alignment code is deleted. T4 onward is not started. The plan builds on actions v1 (`press` and `edit` events, plan.md §4). The engine section below replaces workflows as the way Sextant finds automation.

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
- **What models see:** see the engine below.

### L3. Evaluation

- **Labels from the person:** the Workflows screen asks "Is this one task?" and "Is this name right?". Answers are corrections (§7.12) and also labels for measuring T2 and T3 on real data.
- **Fixtures:** 10 task types with hand-written skills. An ignored test runs the bridge and scores step precision and recall.

## Engine: a memory your agent can read

Status: decided 2026-11-01. E1, D1, E2, E3, E4a and E4b are built. It replaces the agent bridge's brief and answer in `core/src/agent.rs`, and the earlier plan of a one-shot task digest. Today the brief holds app, category and minutes per step of the workflows the rules found. That is too little for an agent to judge, and it hides every routine the rules miss.

Code builds the graph and the day pages, local models label and embed, your Claude Code or Codex reasons over them through MCP tools, and code checks every claim. The model proposes, code measures.

### What the research says

- **Compile, don't summarize.** [Activity Frames](https://arxiv.org/html/2608.05784) compiles a day of screen activity with rules. Sonnet 4.5 and Opus 4.5 answered 98.4 % of questions right from that page, against 66.1 % and 80.4 % from a model-written day summary. The summaries got durations wrong by 25 to 136 %, the compiled page by 7.3 %, and the page was 86 times smaller than the raw rows.
- **Search beats graphs alone.** In [AgentMemBench](https://arxiv.org/pdf/2608.00009), dense retrieval was the key ingredient of long-term agent memory. Graph and summary strategies scored below it, and graph retrieval suffered from noisy entity extraction.
- **Keep provenance.** [Graphiti](https://vectorize.io/articles/zep-vs-cognee) ties every fact to its source and marks outdated facts invalid instead of deleting them.
- **Segmentation is the open problem** in robotic process mining ([Leno et al.](https://link.springer.com/article/10.1007/s12599-020-00641-4)). Sextant's identifier anchors are its answer.

So day pages are compiled by code, graph edges come only from exact anchors (ids, emails, amounts, copied values, time order), and embeddings add search and "similar" edges.

### Layers

```
 L0 raw = rewind   events, screen lines, transcripts          exists, encrypted
       │  code: split into activities, pull out anchors
       ▼
 L1 graph          action ─in─▶ activity ─part of─▶ task
                   activity ─next (gap, value copied)─▶ activity
                   activity ─mentions─▶ entity (person, email, id, amount, document)
                   activity ─similar─▶ activity                 (local embeddings)
       │  code compiles
       ▼
 L2 pages          day and week: time per task, entities touched, values
                   moved between apps, open promises, capture gaps
       │  Claude Code or Codex through MCP tools
       ▼
 L3 routines       the agent proposes and cites ids, code checks them
                   ─▶ SKILL.md ─▶ Claude Cowork, Codex
```

Names, smallest first: an **action** is one press or edit, an **activity** is one focus span on one object, a **task** is activities joined by a shared anchor, and a **routine** is tasks that repeat.

### Who does what

| Job                                       | Who                                                  | Why                                                            |
| ----------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------------- |
| Split, link, count, compile pages, verify | Code                                                 | Exact, testable, fast                                          |
| Short labels for activities and tasks     | Apple Foundation Models (`app/swift/Language.swift`) | Free and private. One activity fits its 4,096-token window.    |
| Search and "similar" edges                | Qwen3-Embedding-0.6B on the bundled ONNX Runtime     | Dense retrieval was the strongest single part in AgentMemBench |
| Find routines across weeks, write skills  | Your Claude Code or Codex                            | Needs reasoning over many days                                 |

A local chat model such as Qwen 4B is not planned: it is a 4 GB download, and the Apple model covers labels.

### What the agent can read and write

One MCP server is the whole engine interface. The bridge, Claude Code, Claude Desktop, Cowork and Codex all call the same tools, so a routine found in any of them lands in the same list. There is no separate brief to keep in sync.

| Tool                   | Returns                                                                                                                                            |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `week(start)`          | Per day: active minutes and top tasks. Across the week: task kinds and activity labels that repeat on 2 or more days, with minutes and days        |
| `day(date)`            | Activities in time order (id, time, minutes, app, label, task), and tasks with their minutes                                                       |
| `task(id)`             | Its activities, its actions run-length compressed with value types, and redacted values when screen text is on                                     |
| `search`, `context_at` | Exist today: full-text search, and the screen text and speech around a moment (rewind)                                                             |
| `save_routine(...)`    | The one new write. Code checks the cited ids, measures hours a week and stores the routine with the time spans it cites. An error says what to fix |
| `routines()`           | Saved routines with their hours and evidence                                                                                                       |
| `check_skill(...)`     | Checks a draft `SKILL.md` for a routine: every numbered step cites `(task <id>: <action>)`, a task the routine cites and an action it holds        |

- Ids survive refreshes: an activity's id comes from its start time and object, and a task's id is its first activity's. A routine stores the time spans it cites, so it outlives the 90-day window.
- The two existing switches stay. "Screen text" (`mcp_rewind`) gates `search`, `context_at` and the values in `task`. "Work patterns" (`mcp_workflows`) gates the rest. Excluded apps never appear, the page tools hold no transcripts, and every result goes through `redact.rs`.
- MCP prompts `find_routines` and `write_skill` hold the instructions, so `/mcp__sextant__find_routines` in Claude Code and a bridge run use the same text.

### E1. Read the graph

- Stable ids. `core/src/pages.rs` compiles `week`, `day` and `task`, and the MCP server lists them in the work patterns group. Their output says activity.
- Done when tests show the fixture week lists the invoice task kind on every day it ran, a day page of the 90-day fixture stays under 8,000 bytes (about 2,000 tokens) and builds in under 100 ms, ids survive a refresh whose window moved, excluded apps are absent, and values are gone with screen text off.
- It ships value alone: a user asks their Claude Code "what should I hand over?" and it reads Sextant's pages.

### E2. Save routines

- `save_routine({title, task_ids, repeats, how, trigger, approval, why})`, `routines()` and the `find_routines` prompt. `repeats` cites work outside tasks as an app and label with the dates it ran, because `day` folds those activities without ids. `approval` names the action to stop before. The model gives no hours and no confidence.
- Code checks: every cited task and repeat exists, the citations hold 3 or more activities on 2 or more days, and the routine shares at most half its time with a saved one. Hours a week are the cited active minutes divided by the weeks of data.
- `schema_v8.sql` adds `routines` and `routine_spans(routine_id, start_ts, end_ts)`.
- Done when a call with an unknown id gets an error naming it, an inflated routine comes back measured, and a duplicate is refused with the id of the saved one.

### E3. The bridge runs your agent on the tools

- `claude -p` runs with `--mcp-config` (Sextant only), `--strict-mcp-config` and `--allowedTools` limited to Sextant's tools. `codex exec` runs with `--ignore-user-config` and `-c mcp_servers.sextant.command=...`. Both run in an empty temp folder with the `find_routines` prompt, and stop after 15 minutes.
- The agent screen lists routines in place of ideas. plan.md §10, the privacy principle in `AGENTS.md` and the README say the agent now reads pages through tools.
- Done when ignored tests run real Claude and real Codex on the fixture and both save the invoice routine.

### D1. Debug report

"Copy debug report" in Settings, for users to paste into a GitHub issue. No network code. It holds the app and macOS versions, permissions, which sources are on, row counts per table and event kind, the last capture time, refresh and capture timings, and the last 20 errors as kinds without text. Done when a test proves the report holds none of the fixture's text.

### E4. Skills, then remove the old path

E4 ships in two steps, because the old path still drives the Workflows and Results tabs, the popover's hours, insights, the share image and corrections.

- **E4a, built:** each routine on the Ideas tab shows a one-line prompt to copy: `/mcp__sextant__write_skill <id>` for Claude Code, and a plain request for Codex, whose support for MCP prompts is unclear. The `write_skill` prompt has the agent read the routine's evidence through `task`, check its draft with `check_skill` and save a `SKILL.md` where it loads skills. Every step cites a task id and an action symbol. An ignored test runs real Claude on the fixture, and every citation resolves.
- **E4b, built:** the main window shows the routines list in place of the Workflows and Results tabs, and the popover drops its Ideas tab. The routines list shows the sum of the routines' hours a week. `schema_v9.sql` drops workflows, follows, commitments, overrides, corrections, handoffs, judgments and `agent_ideas`, along with `list_workflows`, `get_workflow`, `get_skill`, `report_handoff`, the judge and the share image. The `steps` table stays without categories, because its names label activities on the week and day pages. Copy and Reminder events no longer reach the finder. Before/after results can come back once E5 matches new tasks to a routine.
- **Activities renamed, built:** `schema_v9.sql` renames `episodes` to `activities`, `task_episodes` to `task_activities` and every `episode_id` to `activity_id`. The code follows.
- Done when a fixture routine yields a skill whose citations all resolve, and every test passes after the removal.

### E5. Search with local embeddings

- Model, decided 2026-11-02: EmbeddingGemma 300M from `onnx-community/embeddinggemma-300m-ONNX`, int8 (`model_quantized.onnx` plus its 309 MB `.onnx_data`), pinned by commit and checksum in `models.rs` like the speech models. MTEB Multilingual v2 is 61.15 at 768 dims and 59.68 at 256 dims, so vectors keep 256 dims, renormalized. Qwen3-Embedding-0.6B scores about 64.3 but has no official ONNX export and needs twice the RAM. The activations break in fp16, so it runs fp32 or int8 on CPU. Queries use the prefix `task: search result | query: `, documents `title: none | text: `. The Gemma license applies to the download, so the download screen names it.
- Loading, decided 2026-11-02: like ASR. Load the model for refresh and on the first search, keep it warm, unload after 2 minutes idle, so RAM without models stays under 150 MB. Add a "with embeddings loaded" row to plan.md §11 with the measured number, and report how far the first search after idle misses 100 ms.
- It runs through the ONNX Runtime the speech models already load (`app/src/platform/speech.rs`, `parakeet_rs::ort`), with one shared environment.
- Vectors live in a BLOB column. Search scans them in Rust and merges the result with FTS5. No sqlite-vec: it is pre-v1, and [an open issue](https://github.com/asg017/sqlite-vec/issues/12) reports it failing with SQLCipher.
- Activities above a similarity threshold get a `similar` edge.
- Done when search finds the fixture's paraphrased queries that FTS5 alone misses, the search budget in plan.md §11 holds, and `scripts/check-network.sh` passes.

## Order

E1 first: it is the smallest step that lets an early adopter's own agent read the graph. D1 comes next, before the public launch. Then E2 to E5. Each step leaves Sextant working, and the old workflow path stays live until E4. T4 and T5 follow, and T4's input classes then join `task`.

| Step | Delivers                                            | Done when                                                    |
| ---- | --------------------------------------------------- | ------------------------------------------------------------ |
| E1   | Stable ids, `week`, `day` and `task` tools          | See E1                                                       |
| D1   | Debug report, no network                            | See D1                                                       |
| E2   | `save_routine`: the agent proposes, code checks     | See E2                                                       |
| E3   | The bridge runs Claude Code or Codex on the tools   | See E3                                                       |
| E4   | Skills, old workflow path removed, episodes renamed | See E4                                                       |
| E5   | Local embeddings, hybrid search, similar edges      | See E5                                                       |
| L1   | On-device labels for activities and tasks           | Labels on the fixture without network                        |
| T1   | Activities with actions and identifiers             | The fixture shows actions per activity                       |
| T2   | Tasks, "where time goes"                            | Pair agreement ≥ 0.9 on the interleaved fixture              |
| T3   | Task types and procedures (deferred)                | The invoice type found with its steps and for-each           |
| T4   | Input classes                                       | Transfers found on the fixture                               |
| T5   | Incremental refresh                                 | Equals full rebuild, under 50 ms                             |
| L2   | Connect to ChatGPT, `.mcpb`                         | **(user)** Claude Desktop and ChatGPT desktop list the tools |

## Open decisions

1. **Swift helper for Apple's on-device model (L1):** decided, and shipped. `app/swift/Language.swift` is compiled by `app/build.rs` and called from `app/src/platform/language.rs`. Without Apple Intelligence it returns nothing and the caller falls back to rules.
2. **Keyboard monitor** for Enter, Tab, Escape and modifier shortcuts only. Without it, "send with Enter" and shortcut-only steps are missing from procedures. It changes the "never keyboard hooks" rule in §4 and needs Input Monitoring permission.
3. **What the agent reads:** decided. The page tools in "What the agent can read and write" return labels, action symbols, value types and redacted identifiers. Field values, `search` and `context_at` follow the screen text switch in Settings.
