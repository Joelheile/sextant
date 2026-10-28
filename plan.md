# Sextant plan

Sextant is a tiny local app for macOS (Windows later). It does two things:

1. **Easy rewind for your agents.** Everything you saw and said becomes searchable text that Claude (or any MCP client) can query: "what did Anna say about pricing on Tuesday?", "find the invoice number I saw yesterday".
2. **Automation finder.** It maps your work into workflows across apps and days and tells you which ones to hand to an agent, then measures the hours you got back.

Everything stays on the device, encrypted. It should feel invisible: no fan, no battery hit, no setup.

**How to use this file with Claude Code:** start each milestone in a fresh session in plan mode and say "Read plan.md. Do milestone M<n> only. Plan first, build, prove every 'done when' item with command output, then commit."

## 1. Principles

- **Value in 60 seconds.** On install, Sextant reads history you already have and shows a first workflow map before recording anything.
- **The agent is the main interface.** Rewind lives in Claude through MCP. The app window is small: a workflow map, a search box, privacy settings.
- **Lean beats complete.** Text only, event driven, one process, one database file, two Rust crates. Every feature has to earn its CPU and its lines of code.
- **Explain everything.** Every score shows its reasons and evidence. No LLM ranks or summarizes inside Sextant. The agent does the language work.
- **Private by construction.** No pixels or audio on disk, no network except one consented model download, MIT license forever.

## 2. Stack

| Choice                                                                                                         | Why                                                                                                                                            |
| -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Tauri 2 + Rust                                                                                                 | The hot path is native anyway. Small download, low idle RAM, the window is destroyed when closed. Windows later without a rewrite.             |
| Two crates: `sextant-core` (dir `core/`, lib `sextant_core`) and `sextant-app` (dir `app/`, binary `sextant`)  | All logic without OS APIs lives in core and is tested on Linux, in CI and in cloud agents. Platform code sits behind traits in the app.        |
| React 19 + TypeScript + Vite, Bun for tooling                                                                  | Plain React, plain CSS, no component or state library.                                                                                         |
| Built-in OCR: Apple Vision (later `Windows.Media.Ocr`)                                                         | Ships with the OS, no download, runs on the Neural Engine. Open OCR models are slower per frame on a laptop.                                   |
| Parakeet TDT 0.6B v3 int8 via `parakeet-rs` (CoreML)                                                           | Smallest open model with German and English (25 languages), much faster than Whisper. Downloaded once after consent. CC-BY-4.0, credit NVIDIA. |
| SQLite with SQLCipher and FTS5 (`rusqlite`, `bundled-sqlcipher-vendored-openssl`), key in Keychain (`keyring`) | SQLCipher encrypts the whole file. FTS5 is SQLite's built-in full-text index. One file, fast search for years.                                 |
| Work graph as SQLite tables + `petgraph` in memory                                                             | Steps, entities and "what follows what" are edges. No graph database.                                                                          |
| MCP over stdio from the same binary (`sextant mcp`)                                                            | Claude Desktop, Claude Code and any MCP client can use it. No network port.                                                                    |

Pre-approved crates for core: rusqlite, thiserror, regex, serde, serde_json, petgraph, insta, criterion. Ask before adding others there. Pin exact versions for capture, OCR, audio and ML crates.

## 3. Sources

| Source                                                                                                                                                                                        | When                        | Default                                       |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------- | --------------------------------------------- |
| **Backfill:** browser history (Chrome, Arc, Brave, Edge; Safari if Full Disk Access is granted), calendar and reminders (90 days), git commits in folders the user picks, recently used files | once at install, then daily | on, each source can be unticked in onboarding |
| **Live:** focused app, window title, document, browser URL                                                                                                                                    | while working               | on                                            |
| **Live:** OCR text of the focused window                                                                                                                                                      | while working               | on                                            |
| **Live:** mic and system audio transcripts                                                                                                                                                    | during calls and voice use  | on, tray shows it, one click stops it         |
| **Live:** copy events (which app, never the content)                                                                                                                                          | while working               | on                                            |
| Screenshots or audio files on disk                                                                                                                                                            | never                       | never                                         |
| Retention of raw data                                                                                                                                                                         |                             | 90 days; 30, 365 or forever                   |

A call is detected when another process (not Sextant) uses the mic and it is a known call app (Zoom, Teams, Meet, Slack, FaceTime, Webex, Discord) or the user marked it. Note for Germany: transcribing others without consent is a crime under §201 StGB, so the onboarding tells users to let the other side know.

## 4. Live capture pipeline

```
triggers ─▶ capture ─▶ diff gate ─▶ OCR ─▶ redact + dedupe ─▶ writer (flush 1 s) ─▶ SQLCipher
mic + system audio ─▶ 30 s ring buffer ─▶ VAD ─▶ Parakeet ─▶ writer
every 5 min: sextant_core::refresh(now) ─▶ episodes ─▶ work graph ─▶ scores
```

- One process, plain threads for capture and audio, `crossbeam_queue::ArrayQueue::force_push` between them (oldest frame drops under load).
- **Triggers:** focus change (300 ms settle), input pause (1 s after activity, read from OS idle time, never keyboard hooks), 10 s tick while active. Nothing when idle over 60 s, locked or asleep.
- **Gates before any frame or stream exists:** excluded app (password managers, Health, messengers by default), private browser window (Chromium `mode`, Arc `incognito`; Safari excluded unless the user opts in), secure input (`IsSecureEventInputEnabled`), paused, source off.
- **Capture:** only the focused window, `SCScreenshotManager` (macOS 14+).
- **Diff gate:** 64×64 grayscale hash, skip OCR if nearly equal to the last frame of that window. Tune with synthetic frames in tests.
- **OCR:** Vision `VNRecognizeTextRequest`, compare `.fast` and `.accurate` against the budget, drop confidence < 0.3. Output per line: text, confidence, box.
- **Redaction before storage:** emails, IBANs, card numbers, API keys, tokens. Emails become `<email:hash>` so entity linking still works.
- **Audio:** per-process mic detection (`kAudioProcessPropertyIsRunningInput`, skipping Sextant), mic via `cpal`, system audio via a Core Audio process tap on the call app only (macOS 14.2+), 16 kHz mono, Silero VAD, Parakeet loaded on first speech and unloaded after 5 min of silence. Mic and system are separate streams, so "me" vs "them" is known without diarization.
- Timeouts on every OS call that can hang (AX 0.5 s, AppleScript 2 s).

## 5. Storage

- `lines(id, hash UNIQUE, text)` each distinct line once, plus `lines_fts` (FTS5, `unicode61`, `remove_diacritics 2`).
- `frames(id, ts, window_id, packed)` varint list of `(line_id, box)`, boxes on a 256 grid.
- `frame_lines(line_id, frame_id) WITHOUT ROWID` for search and cleanup.
- `windows(id, app, bundle_id, title, doc, url_host_path)` deduped.
- `utterances(id, ts_start, ts_end, source mic|system, text)` plus `utterances_fts`.
- `events(ts, kind, source, json)` for focus, idle, copy, calendar, reminder, browser visit, commit, file use, pause. `source` is `live` or `backfill`.
- Engine: `episodes`, `commitments`, `overrides`, `corrections`, `handoffs`.
- Work graph: `steps`, `entities`, `mentions(entity_id, episode_id)`, `follows(from_step, to_step, count, median_gap_s, copies)`, `cases(entity_id, first_ts, last_ts, variant)`, `workflows(id, variant, case_count, hours_week)`.
- Core has a passive `Writer` (`push`, `flush(now)`); the app's writer thread flushes every second. Readers (UI and MCP) use read-only connections, WAL mode lets them run while the app writes.
- **Retention, hourly:** delete raw rows past the window, then unreferenced lines and windows. Weekly `VACUUM` when idle. Workflows and scores keep only redacted, templated keys.
- Dev builds use `SEXTANT_HOME=./.dev-data` (in `.cargo/config.toml`) and keychain service `sextant-dev`, and refuse to start without it.

## 6. Backfill (the 60-second first map)

- Browser history: copy the `History` SQLite file to memory first (the browser locks it), read `urls` and `visits` (`visit_time`, `visit_duration`, WebKit epoch). Group visits into sessions by host and 5 min gaps.
- Calendar and reminders: EventKit, last 90 days and next 14.
- Git: `git log --all --since=90.days --format=%H%x09%at%x09%s` per repo in folders the user picks.
- Recent files: Spotlight `kMDItemLastUsedDate` for documents, file names only.
- Everything becomes `events` with `source = backfill` and goes through the same compiler. Workflows built only from backfill are labeled "rough" until live data confirms them.

## 7. Automation finder (core, deterministic, time passed in as `now`)

Process mining applied to one person: a **case** is one piece of work (one client, ticket, invoice), a **step** is a recurring activity, a **workflow** is a step sequence that repeats across cases.

1. **Episodes:** consecutive focus on the same object key, min 5 s, max 120 min, merge gaps under 2 min. Object key: host + first 2 path segments + 3 title tokens; else app + document; else app + title template (numbers and ids replaced). OCR headings and form titles refine the key.
2. **Steps:** group episodes by object key, merge within the same app or site at Jaccard ≥ 0.5 on title tokens. Name from shared tokens. Category by rules: meeting, email, spreadsheet, crm, finance, chat, planning, coding, design, writing, slides, calendar, social, media, web, other.
3. **Entities** (rules, no LLM): ticket and invoice ids, email hashes and domains, document names, amounts with currency, names or companies seen in 3+ episodes. Ignore entities that appear almost everywhere.
4. **Follows edges:** B starts within 10 min after A ends. Keep count, median gap and copy events from A's app to B's app.
5. **Cases:** episodes that mention the same entity, in time order, across days.
6. **Workflows:** group cases by step sequence (edit distance ≤ 1), 3+ cases. Frequent follows chains without a shared entity (count ≥ 5 on 3+ days) are routine workflows.
7. **Commitments:** promises in transcripts ("I'll", "let me", "ich werde", "ich schicke", "mach ich") and reminders, linked to cases by entity, else token overlap ≥ 0.34.
8. **Scores:**
   - hours per week = minutes / 60 / observed days × 5
   - step potential = category prior × length factor (≤ 60 min 1.0, ≤ 180 min 0.85, else 0.75) × (0.6 + 0.4 × stability) × repetition (2+ days 1.0, else 0.8). Priors: crm .75, spreadsheet .7, calendar .7, finance .65, email .6, planning .55, coding .55, web .5, writing .45, slides .4, social .35, other .35, chat .3, design .2, meeting .1, media .05
   - workflow potential = time-weighted mean of steps, + 0.1 if data is copied by hand between steps (cap 0.95)
   - desire from user enjoyment: hate 1, dislike .9, neutral .7, unknown .6, like .35, love 0
   - estimated benefit = hours per week × potential × desire, halved for "assist"
   - waiting = median case length minus active time; procrastination = overdue commitments × 0.35 + abandoned opens × 0.15 + 0.1 per waiting day over 2 (cap 1)
9. **Kind:** friction (data copied by hand, or 12+ app switches per hour), promise gap (procrastination ≥ 0.4), routine (3+ cases or steady daily repetition), long tail (< 0.5 h per week).
10. **Action:** love → protect; hobby or media → ignore; mostly meetings → assist; else automate if potential ≥ 0.6 and ≥ 0.5 h per week. Name the best first step to hand off.
11. **Ask first:** the 3 biggest workflows with unknown enjoyment.
12. **Corrections, rules only, English and German:** "I hate this", "it's for a client", "can't be automated", "rename to …", "hide this", "these belong together", "this is not one workflow". Stored per step key and variant.

## 8. MCP (the agent interface)

`sextant mcp` runs the same binary as a stdio MCP server with a read-only DB connection. Onboarding has "Connect to Claude": it adds Sextant to Claude Desktop's config and shows the one-line command for Claude Code (`claude mcp add sextant -- <path>/sextant mcp`).

| Tool                                | Returns                                                                                  |
| ----------------------------------- | ---------------------------------------------------------------------------------------- |
| `search` (query, from?, to?, app?)  | matching screen lines and transcript lines with time, app, window, grouped into sessions |
| `context_at` (time, window?)        | the screen text and transcript around a moment                                           |
| `timeline` (from, to)               | episodes and meetings in that range, compact                                             |
| `list_workflows`                    | ranked workflows with hours, potential, reasons                                          |
| `get_workflow` (id)                 | steps, follows, example cases (redacted), commitments                                    |
| `get_skill` (id)                    | ready `SKILL.md` for that workflow                                                       |
| `report_handoff` (id, status, note) | records that an agent took over a workflow, starts the before/after measurement          |

- Results are capped (default 50 lines) and paginated, so they fit an agent's context.
- Whatever the agent uses runs where its model runs, maybe in the cloud. The connect screen says so plainly. Excluded apps and redaction apply to MCP output too. Rewind tools and workflow tools can be switched separately.

## 9. App UI (small)

- Tray icon while capturing (filled on, outline paused, dot while transcribing). Menu: status, Pause 1 h, Open, Quit.
- **Onboarding** (one screen per step): pick backfill sources, grant permissions, see the first map, connect to Claude.
- **Workflows:** "X hours a week you could get back", quick enjoyment question, ranked workflows as rows of steps (apps, gaps, copy points), promises you keep pushing, things to protect. Detail: reasons, example cases, correction box, "Create automation" (writes `SKILL.md`, copies a ready prompt for Claude).
- **Search:** one box over screen text and transcripts, results by session. No timeline replay.
- **Results:** hours before and after each handoff, from 3+ days of data.
- **Privacy:** sources and permissions, excluded apps, retention, storage used, MCP tool switches, "Delete everything".
- Plain CSS, light and dark, English and German strings, short copy, no em dashes. Data only through typed Tauri commands, paginated. CSP `default-src 'self'; connect-src ipc: http://ipc.localhost`, no `shell`, `http` or `updater` plugins.

## 10. Privacy rules (non-negotiable)

- Pixels and audio never touch disk, logs or panic messages. Only test fixtures may hold images or audio.
- Only the focused window, only after all gates pass.
- Network code only in `app/src/models.rs` (model download with pinned URL and SHA-256 after consent). A CI script fails on network crates or `std::net` anywhere else, checked with `cargo tree --target all`.
- No telemetry, crash reporter, auto-updater or analytics.
- Logs hold counts and timings, never text, titles, URLs or transcripts.
- Enjoyment and value only from the user. Never infer emotions.
- Claude Code never reads the real data folder (`~/Library/Application Support/Sextant`) or the `sextant` keychain entry.

## 11. Budgets (the definition of lean and fast)

Measured by `sextant bench` on fixtures and `sextant bench --live 3600` on a real session (counts and timings only). Reference: M1 MacBook Air.

| Metric                                              | Budget                        |
| --------------------------------------------------- | ----------------------------- |
| First workflow map after install (90 days backfill) | < 60 s                        |
| CPU idle / working (1 h average)                    | < 0.5 % / < 3 %               |
| RAM without / with ASR loaded                       | < 150 MB / < 1.3 GB           |
| OCR per window, p95                                 | < 250 ms, off the main thread |
| Capture to searchable                               | < 5 s                         |
| Storage per workday                                 | < 15 MB                       |
| Search over 1 year, p95                             | < 100 ms                      |
| MCP tool response, p95                              | < 200 ms                      |
| Full refresh over 90 days                           | < 2 s                         |
| App download without models                         | < 30 MB                       |
| ASR real-time factor                                | ≤ 0.2                         |

If a budget can't be met, stop and report numbers and options. Never loosen one silently.

## 12. Working rules for the agent

- One milestone per session. Plan, build, prove. Bug fix: failing test first.
- Logic goes in core whenever it needs no OS API. Platform code implements a trait with `macos` and `fake` impls (`fake` replays fixtures so gate tests run without permissions). `#[cfg]` at module level only.
- Never block the Tauri main thread. Never hold a DB transaction across a capture or OCR call.
- Rust 2024, `thiserror` in core, `anyhow` in app, no `unwrap()` outside tests. Snapshots: read `.snap.new`, then `cargo insta accept`.
- Prefer one more function over one more crate.
- Before calling a milestone done, run a review subagent with only this file and the diff (`git add -N . && git diff HEAD`) against §10.

## 13. Milestones

**(user)** marks steps a human must do. The agent prepares them and waits.

**M0 Skeleton.** Workspace with the two crates, Tauri app with tray and empty React window, `.cargo/config.toml`, `app/Info.plist` and `app/Entitlements.plist` (§14), a short `CLAUDE.md` that points here and lists the commands, a Claude Code Stop hook that runs fmt, clippy, tests and the network script (exit 2 on failure), CI (core on ubuntu, app on macos-15).
Done when: the hook passes; **(user)** CI green, tray icon visible.

**M1 Core.** Synthetic fixtures with a fixed date and timezone (a week of events, 5 invoices each spread over 2 to 4 days across email, spreadsheet and lexoffice, noise in between, English and German OCR samples), store and migrations, dedupe, FTS search, retention, work graph, finder, corrections, `SKILL.md` builder, `insta` golden tests.
Done when: tests pass on Linux, the finder recovers the invoice workflow, search p95 < 100 ms on a generated 1-year DB, refresh over 90 days < 2 s.

**M2 Backfill + MCP.** Backfill readers, onboarding with first map, MCP server with all tools, "Connect to Claude".
Done when: first map < 60 s on a 90-day fixture profile; an MCP client test calls every tool against a fixture DB within budget; **(user)** Claude Desktop answers a question from your real backfill.
**Validation gate (user):** give this build to 10 people, walk each through their map, count who hands a workflow to an agent within a week. Decide on M3 after that.

**M3 Live capture.** Focus metadata, browser URL, triggers, gates, window capture, diff gate, Vision OCR, writer.
Done when: fake-platform tests prove gated sources write zero rows; OCR budget met on fixture images; **(user)** live bench meets CPU, RAM and storage.

**M4 Audio.** Per-process mic detection, mic and system audio, VAD, Parakeet, tray indicator with one-click stop, consented model download.
Done when: real-time factor ≤ 0.2 on speech fixtures (LibriSpeech and Multilingual LibriSpeech, fetched by a script into a git-ignored folder); no audio in the data folder after a session; switching audio off stops streams within 1 s.

**M5 Loop and UI.** Workflows, Search, Results and Privacy screens; `report_handoff`; before and after measurement; weekly report with a redacted share image.
Done when: corrections change scores on fixtures, a reported handoff shows hours before and after, "Delete everything" leaves an empty data folder.

**M6 Release.** Signing with hardened runtime, notarization, DMG, GitHub release, one-line install in the README.
Done when: **(user)** a fresh Mac installs, grants permissions and shows a map without a terminal.

**Later.** Windows (Windows Graphics Capture, `Windows.Media.Ocr`, process loopback, UI Automation; same traits). German workplace mode (work hours, 7-day retention, per-call consent, MCP limited to workflows, works council docs) as the paid team product.

## 14. macOS bundle

`minimumSystemVersion` 26.0, so the search palette can use Apple's on-device model (Foundation Models).

- `Info.plist`: `NSAudioCaptureUsageDescription`, `NSMicrophoneUsageDescription`, `NSAppleEventsUsageDescription`, `NSCalendarsFullAccessUsageDescription`, `NSRemindersFullAccessUsageDescription`, `LSUIElement`.
- `Entitlements.plist`: `com.apple.security.device.audio-input`, `com.apple.security.automation.apple-events`, `com.apple.security.personal-information.calendars`.
- Permissions asked only when the matching step in onboarding needs them: Screen Recording (OCR), Accessibility (window titles), Automation (browser URL), Calendars and Reminders (backfill), Microphone and System Audio (transcripts), Full Disk Access (Safari history, optional).
