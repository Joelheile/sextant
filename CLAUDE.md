# Sextant

Read `plan.md` first. It is the spec: principles, privacy rules (§10), budgets (§11), working rules (§12) and milestones (§13). Do one milestone per session.

## Layout

- `core/`: crate `sextant-core`, all logic without OS APIs. Tested on Linux.
- `app/`: crate `sextant-app`, binary `sextant`. Tauri 2, tray, platform code.
- `ui/`: React 19 + TypeScript window, built by Vite into `dist/`.
- `scripts/`: `check-network.sh` (privacy check), `stop-hook.sh` (runs every check).

## Commands

| Task | Command |
|---|---|
| Install JS deps | `bun install` |
| Run the app | `bun tauri dev` |
| Build the UI | `bun run build` |
| Format | `cargo fmt --all` |
| Lint | `cargo clippy --workspace --all-targets -q -- -D warnings` |
| Test | `cargo test --workspace -q` |
| Test core only | `cargo test -p sextant-core -q` |
| Budget tests (first map, MCP p95, search, refresh) | `cargo test -p sextant-core --release -- --nocapture` |
| MCP server over stdio | `SEXTANT_HOME=.dev-data cargo run -q -- mcp` (or `sextant mcp` from a build) |
| Network check | `scripts/check-network.sh` |
| All checks (Stop hook) | `scripts/stop-hook.sh` |

## Rules

- Dev builds read and write only `./.dev-data` (`SEXTANT_HOME`, set in `.cargo/config.toml`). Never read `~/Library/Application Support/Sextant`, the Claude Desktop config, or the `sextant` keychain entry. Dev builds use the `sextant-dev` entry; tests set `SEXTANT_DEV_KEY` (64 hex digits, debug only) instead.
- Never run backfill against real browsers, calendar, git or Spotlight in development. Tests use `core/src/profile_fixture.rs` and temp folders.
- Network code only in `app/src/models.rs`.
- No `unwrap()` outside tests. `thiserror` in core, `anyhow` in app.
