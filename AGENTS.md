# Sextant for agents

Read this first, then `CLAUDE.md` for commands and rules, `plan.md` for the spec and `docs/routines-plan.md` for the task graph and LLM layer.

## The problem

Agents like Claude Code, Claude Cowork and Codex can now do much of the work people do at a computer. Most people still hand them very little, for three reasons:

- **You don't know what to hand over.** You pick from memory, and memory misses the repeated work that eats the hours: the invoice copied from Mail into the accounting app, the CRM update after every call, the Monday report built from three tabs.
- **Your agent doesn't know how you do it.** It starts every task blind: which apps, which fields, where each value comes from, what to check before sending. Writing that down by hand takes longer than doing the work.
- **Today's tools don't close the gap.** "Record a skill" needs you to know what to automate and shows one run, so it can't tell the values that change from the ones that don't. Screen recorders such as Screenpipe store raw text and leave the mining to you. Enterprise task mining is sold to employers, costs six figures and mostly runs on Windows.

## Vision

Sextant is procedural memory for your agents. It learns how you work on your Mac, finds the routines worth handing over, measures the hours they take, and gives your agent a skill built from what you actually did.

It aims to be the simplest and cheapest way to get there:

- **Simplest:** install it and forget it. No setup, no recording sessions, no prompts to write. It tells you what to hand over.
- **Cheapest:** free and open source, and cheap to run. No subscription, no cloud, no model calls per event, no noticeable CPU or battery cost. Reasoning runs on the Claude Code or Codex plan you already pay for.

Every choice is judged by this. When a feature makes Sextant harder to use or more expensive to run, it needs a strong reason to exist.

## Who it is for

- **Now: early AI adopters on a Mac.** Founders, indie builders, and ops, finance or sales people at small companies who already use Claude Code or Codex and want them to take over real work. They install from GitHub, run the agent themselves and give feedback.
- **Later: white-collar workers in general.** Anyone whose week holds repeated multi-app work. Reaching them needs Cowork-style handoff with no terminal, so nothing in the design may assume a developer.
- **Not:** employers who want to monitor staff, or anyone recording other people. Sextant never reports to anyone but the user, and call audio stays off until the user turns it on.

Judge every feature by one question: does it get an early adopter to their first handover sooner?

## The promise

1. **What should I hand over?** Your routines, ranked by the hours an agent could give back, each with the evidence behind it.
2. **Hand it over.** Each routine becomes a `SKILL.md` that Claude Cowork, Claude Code or Codex can run, with every step citing what Sextant saw.
3. **Where does my time go?** Day and week pages by task ("invoice for Acme", "weekly report"), not by app.
4. **Search and rewind your past.** "Find the invoice number I saw yesterday." You and your agent read the same record.

The user does nothing beyond installing and granting permissions. A first map appears within a minute, from history already on the Mac.

## How we measure it

| Measure    | Definition                                                     |
| ---------- | -------------------------------------------------------------- |
| Aha moment | A notification names a routine you recognize, with its hours. At most one a week, with an off switch |
| Activation | The first routine handed to an agent within 14 days of install |
| North star | Hours a week handed to agents that still run after 30 days     |

The M2 validation gate in `plan.md` §13 (10 people, count handovers within a week) has not run yet.

## Principles

- **Private by construction.** Text only. Pixels, audio and keystrokes never touch disk. One encrypted database. No telemetry, no cloud, network code in one file and a CI check that enforces it. Data leaves the Mac only when the user's own agent reads it. The agent bridge gives Claude only Sextant's MCP tools. Codex has no such mode, so it runs `exec --sandbox read-only` with the network off, without user config or rules, and with web search, apps, plugins, the browser, computer use and hooks off (`plan.md` §10).
- **Semantic capture beats pixels.** macOS Accessibility already says which button was pressed and which field was filled. Sextant reads that instead of running a vision model on every event, so it costs no model calls per event and can run all day.
- **Code compiles, models judge.** Code builds the graph and the day pages and checks every claim a model makes. Agents answer questions about a day more accurately from a page compiled by code than from a summary a model wrote ([Activity Frames](https://arxiv.org/html/2608.05784)). Apple's on-device model writes labels. A local embedding model powers search. Your Claude Code or Codex finds routines and writes skills.
- **Explain everything.** Every score shows its reasons and evidence. Every skill step cites what was observed. No ungrounded step ships.
- **Invisible.** No fan, no battery hit, no setup. Every feature earns its CPU and its lines of code.
- **The agent is the main interface.** The app window stays small. Claude reaches the data through MCP.
- **Trust you can check.** Every claim has a place to see it: the sent log lists each answer an agent got, the tray says when capture stops, excluded apps and sites drop data before it is stored, and you can delete the last hour or uninstall in one click.

## Words

| Word     | Meaning                                                                   | Example                      |
| -------- | ------------------------------------------------------------------------- | ---------------------------- |
| action   | One press or edit: app, role, label template                              | Mail · AXButton · send       |
| activity | One focus span on one object, with its actions (code: `episode` until E4) | 8 minutes on one Mail thread |
| task     | Activities joined by a shared anchor (invoice number, email, amount)      | Invoice 4711 for Acme        |
| routine  | Tasks that repeat, found by your agent and checked by code                | Monthly invoicing            |

## How it works

```
 raw = rewind   events, screen lines, transcripts (encrypted SQLite)
     │  code
     ▼
 graph          actions, activities, tasks, entities; edges for time order,
                copied values, mentions and similarity
     │  code
     ▼
 pages          day and week: time per task, values moved between apps
     │  your Claude Code or Codex, through Sextant's MCP tools
     ▼
 routines       the agent proposes and cites ids, code checks and measures
     ▼
 skills         SKILL.md to Claude Cowork, Claude Code and Codex
```

| Layer              | What it holds                                                                                                                                 | Where                                     | Status                      |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- | --------------------------- |
| Capture            | Focused app, window, URL, screen text, presses on labelled controls, the value left in each text field, meeting transcripts. Gated, redacted. | `app/src/platform`, `core/src/capture`    | shipped                     |
| Raw store          | Events, screen lines and transcripts, with full-text search. This is rewind.                                                                  | `core/src/store.rs`, `core/src/search.rs` | shipped                     |
| Activities (T1)    | Focus spans with their actions. Values get a type (email, id, amount, date).                                                                  | `core/src/finder/actions.rs`              | shipped                     |
| Tasks (T2)         | Activities joined by shared identifiers. Answers "where does time go".                                                                        | `core/src/finder/tasks.rs`                | shipped                     |
| Pages (E1)         | Week, day and task pages compiled by code, read through MCP tools                                                                             | `core/src/pages.rs`, `core/src/mcp`       | shipped                     |
| Discovery (E2, E3) | Your agent finds routines through the tools and saves them. Code checks every cited id and computes the hours.                                | `core/src/routines.rs`, `app/src/agent.rs` | shipped                     |
| Skills (E4)        | `SKILL.md` per routine with cited steps, copied to where Cowork and Codex load skills                                                         | `core/src/skill.rs`                       | exists for workflows        |
| Search (E5)        | Local embeddings next to full-text search, and "similar" edges between activities                                                             | `app/src/models.rs`, `core/src/search.rs` | planned                     |
| Input classes (T4) | Where each step's values come from: fixed, copied, derived or judgment                                                                        | not yet                                   | planned                     |

Workflows and `score.rs` are the old path. They stay live until E4 works, then go.

## Quality bars

| Measure                                         | Target                               |
| ----------------------------------------------- | ------------------------------------ |
| First map after install                         | under 60 s                           |
| Task grouping vs. ground truth (pair agreement) | 0.9 or more                          |
| Full refresh over 90 days                       | under 2 s                            |
| Incremental refresh                             | under 50 ms, equal to a full rebuild |
| Search p95                                      | under 100 ms                         |
| Clustering 10,000 tasks                         | under 200 ms                         |

If a budget can't be met, stop and report numbers and options. Never loosen one.

Before a PR, run the agent-desktop-testing skill. Its recipe is `.agents/agent-desktop-testing.md`.

## Never

- Store pixels, audio, keystrokes or secure field values.
- Record password managers, private browser windows or secure input.
- Add telemetry, analytics, a crash reporter or an auto-updater.
- Put network code outside `app/src/models.rs`.
- Read the real data folder or keychain entry from an agent, or capture the real screen. Use fixtures and fakes.
- Claim "nothing leaves your Mac". Say it stays on the Mac until the user's agent reads it.
- Let an answer reach an agent without a row in the sent log.
- Require an account or an email, change the MIT license, or sell a paid tier that holds back privacy or core features.

## Distribution

- MIT, open source on GitHub. The README is the landing page.
- Install leads with `curl | sh`, which opens without a Gatekeeper prompt. The DMG is second. Releases are signed with a Developer ID and the hardened runtime, notarized and stapled by the Release workflow (see `docs/RELEASING.md`).
- Name only clients that work today. Claude Desktop and Claude Code work through MCP today. Codex works through the agent bridge. ChatGPT and Codex over MCP come after L2 ships.

## Status

- **Shipped in the tree:** capture with AX-first text, actions v1, signals, popover, activities and tasks, stable ids, the `week`, `day` and `task` pages over MCP (E1), the debug report (D1), `save_routine` (E2) and the agent bridge running your agent on those tools (E3).
- **Next:** the engine in `docs/routines-plan.md`: E4 and E5. Then input classes (T4) and incremental refresh (T5).
- **Deferred:** task types (T3). Your agent finds recurring work in E2 and E3, so nothing needs task types yet.
