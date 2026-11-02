# Agent desktop testing recipe

- Build: `bun install && bun run build` (UI typecheck), then the launch command builds the Rust app.
- Launch: `bun tauri dev` (debug build, `SEXTANT_HOME=.dev-data` from `.cargo/config.toml`).
- App name for --app: `sextant`
- Ready when: `agent-desktop wait --window Sextant --app sextant`
- Reset: stop the app (`agent-desktop close-app sextant`), then `rm -f .dev-data/sextant.db* .dev-data/settings.json`. Keep `.dev-data/models`. A fresh `settings.json` has `onboarded: false`, so the app opens onboarding. The webview keeps the onboarding step in localStorage (`onboardingStep`); press Back to the sources step before ending a run.
- Isolation: dev builds read and write only `./.dev-data` and use the `sextant-dev` keychain entry. No Sextant is installed in /Applications, so `--app sextant` cannot hit a release copy.
- Physical input: not allowed. Run headless only.
- Entry points: main window "Sextant" (onboarding until finished, then tabs), menu bar status item (`--surface menubar`), popover window "Sextant" at layer 5, rewind search window.
- Blind spots:
  - Never press "Read my history" in onboarding or start a backfill: it reads real browsers, calendar, git and Spotlight, which `CLAUDE.md` forbids in development. The map and connect steps are covered by `bun run build` and by reading `ui/Onboarding.tsx`, not driven.
  - System permission prompts and System Settings drag panes (Allow buttons) change real TCC state. Read their labels and state only; do not press them.
  - Live capture starts once the app runs and writes to `.dev-data`; reset afterwards.
