# Sextant for agents

Read this first, then `CLAUDE.md` for commands and rules, `plan.md` for the spec and `docs/routines-plan.md` for the task graph and LLM layer.

## Vision

Build the simplest and cheapest app that finds what to automate in your daily workflows.

- **Simplest:** install it and forget it. No setup, no recording sessions, no prompts to write. It shows you the routines worth handing to an agent.
- **Cheapest:** free and open source, and cheap to run. No subscription, no cloud, no model calls per event, no noticeable CPU or battery cost.

Every choice is judged by this. When a feature makes Sextant harder to use or more expensive to run, it needs a strong reason to exist.

## Who it is for

- **Everyone on a Mac who works at a computer:** ops, finance, sales, founders, consultants, assistants, developers.
- **Early adopters:** developers and people who already use Claude, who install from GitHub and give feedback.
- **Not:** employers who want to monitor staff. Sextant never reports to anyone but the user.

## The promise

1. **What should I automate?** Your daily routines, ranked by the hours an agent could give back, with reasons.
2. **Hand it over.** Each routine becomes a `SKILL.md` that Claude can run.
3. **Where does my time go?** By task ("invoice for Acme", "weekly report"), not by app. This is how Sextant finds the routines.
4. **Search your past.** "Find the invoice number I saw yesterday." A side benefit of the same record.

The user does nothing beyond installing and granting permissions. A first map appears within a minute, from history already on the Mac.

## Principles

- **Private by construction.** Text only. Pixels, audio and keystrokes never touch disk. One encrypted database. No telemetry, no cloud, network code in one file and a CI check that enforces it. Data leaves the Mac only when the user's own agent reads it. The agent bridge runs Claude with no tools. Codex has no such mode, so it runs `exec --sandbox read-only` with the network off, without user config or rules, and with web search, apps, plugins, the browser, computer use and hooks off (`plan.md` §10).
- **Semantic capture beats pixels.** macOS Accessibility already says which button was pressed and which field was filled. Sextant reads that instead of running a vision model on every event, so it costs no model calls per event and can run all day.
- **Algorithms first, models for judgment.** Grouping, clustering and alignment are deterministic and fast. Apple's on-device model writes labels and judges steps. Claude does the language work when the user asks.
- **Explain everything.** Every score shows its reasons and evidence. Every skill step cites what was observed. No ungrounded step ships.
- **Invisible.** No fan, no battery hit, no setup. Every feature earns its CPU and its lines of code.
- **The agent is the main interface.** The app window stays small. Claude reaches the data through MCP.

## How it works

| Layer               | What it holds                                                                                                                                   | Where                                    |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| Capture             | Focused app, window, URL, screen text, presses on labelled controls, the value left in each text field, meeting transcripts. Gated, redacted.   | `app/src/platform`, `core/src/capture`   |
| Activities (T1)     | Episodes of focused work, each with its actions. An action is a symbol: app, role, label template. Values get a type (email, id, amount, date). | `core/src/finder/actions.rs`             |
| Tasks (T2)          | Episodes joined into tasks by shared identifiers (invoice number, ticket id). Replaces cases. Answers "where does time go".                     | `core/src/finder/tasks.rs`               |
| Task types (T3)     | Deferred. Recurring tasks clustered by MinHash over action symbols, aligned into steps and loops. Nothing reads task types yet, so the code is gone. | deferred                                 |
| Input classes (T4)  | Where each step's values come from: fixed, copied, derived, or judgment. The copied and fixed share is what an agent can do.                    | planned                                  |
| Skills and MCP (L2) | `SKILL.md` per routine, MCP tools for Claude Desktop, Claude Code and Codex.                                                                    | `core/src/skill.rs`, `core/src/mcp`      |

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

## Never

- Store pixels, audio, keystrokes or secure field values.
- Record password managers, private browser windows or secure input.
- Add telemetry, analytics, a crash reporter or an auto-updater.
- Put network code outside `app/src/models.rs`.
- Read the real data folder or keychain entry from an agent, or capture the real screen. Use fixtures and fakes.
- Claim "nothing leaves your Mac". Say it stays on the Mac until the user's agent reads it.

## Distribution

- MIT, open source on GitHub. The README is the landing page.
- Install leads with `curl | sh`, which opens without a Gatekeeper prompt. The DMG is second. Releases are signed with a Developer ID and the hardened runtime, notarized and stapled by the Release workflow (see `docs/RELEASING.md`).
- Name only clients that work today. Claude Desktop and Claude Code work through MCP today. Codex works through the agent bridge. ChatGPT and Codex over MCP come after L2 ships.

## Status

- **Shipped in the tree:** capture with AX-first text, actions v1, signals, popover and agent bridge, activities and tasks.
- **Deferred:** task types (T3). `from_tasks` in `core/src/finder/graph.rs` is the one grouping engine until something needs task types.
- **Next:** input classes (T4), incremental refresh (T5), MCP tools and skill citations (L2), evaluation from user corrections (L3).
