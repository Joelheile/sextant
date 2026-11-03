# Agent desktop testing recipe

- Build: `cargo build -p sextant-app`.
- Launch: `cargo run` (debug build, `SEXTANT_HOME=.dev-data` from `.cargo/config.toml`).
- App name for --app: `sextant`
- Ready when: `cargo run -q -- status` prints a status line instead of "Sextant is not running".
- Reset: Quit from the menu bar menu, then `rm -f .dev-data/sextant.db* .dev-data/settings.json`. Keep `.dev-data/models`. A fresh `settings.json` has `onboarded: false`, so the app opens `sextant setup` in Terminal.
- Isolation: dev builds read and write only `./.dev-data` and use the `sextant-dev` keychain entry. No Sextant is installed in /Applications, so `--app sextant` cannot hit a release copy.
- Physical input: not allowed. Run headless only.
- Entry points: menu bar status item (`--surface menubar`), and the CLI and TUI (`cargo run -q -- help`, `cargo run -q -- tui`) against the running dev app.
- Blind spots:
  - System permission prompts and System Settings drag panes (Allow buttons, `sextant setup`) change real TCC state. Read their labels and state only; do not press them.
  - Live capture starts once setup is done and writes to `.dev-data`; reset afterwards.
