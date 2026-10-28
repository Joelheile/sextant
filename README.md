# Sextant

Sextant is a small app for macOS that makes your work searchable and finds the parts worth handing to an agent.

- **Rewind for your agents.** Everything you saw and said becomes searchable text. Claude, or any MCP client, can ask: "What did Anna say about pricing on Tuesday?" or "Find the invoice number I saw yesterday."
- **Automation finder.** Sextant maps your work into workflows across apps and days, shows which ones to hand to an agent, and measures the hours you get back.

On install, Sextant reads history you already have (browser history, calendar, git commits, recent files) and draws a first workflow map in under a minute.

## Install

Sextant needs a Mac with Apple silicon and macOS 26 or later.

```sh
curl -fsSL https://github.com/Joelheile/sextant/releases/latest/download/install.sh | sh
```

The script downloads the latest disk image over HTTPS, checks its SHA-256 against the checksum published with the release, copies `Sextant.app` to `/Applications` and opens it. You can also download `Sextant.dmg` from the [releases page](https://github.com/Joelheile/sextant/releases/latest) and drag the app to `/Applications`.

While the repository is private, the `curl` command can't download anything. Use the GitHub CLI instead: `gh release download -R Joelheile/sextant -p Sextant.dmg`, then open the disk image and drag the app to `/Applications`.

The app is about 17 MB. The speech models (about 670 MB) are optional and download only after you press "Download".

## Privacy

- Everything stays on your Mac, in one encrypted database. The key lives in your Keychain.
- Sextant never stores pixels or audio. Screen text and transcripts are stored as text only.
- It reads only the focused window, and only when no gate is closed: excluded apps (password managers and messengers by default), private browser windows, secure input, paused, or the source switched off.
- Emails, IBANs, card numbers, API keys and tokens are redacted before anything is stored.
- The only network call is the speech model download you start yourself, from pinned URLs with a SHA-256 check. There is no telemetry, crash reporter, analytics or auto-updater.
- You choose how long raw data stays: 30, 90 or 365 days, or forever. "Delete everything" empties the data folder.
- If you tell your agent to read from Sextant, the text it reads goes to wherever that agent's model runs, which may be the cloud. Excluded apps and redaction apply to that output too.

Transcribing people without their consent is a crime in some countries, including Germany (section 201 StGB). Tell the other side before you record a call.

## Connect to Claude

The onboarding has a "Connect to Claude" step. It adds Sextant to the Claude Desktop config, and keeps a backup of the old file. For Claude Code, run:

```sh
claude mcp add sextant -- /Applications/Sextant.app/Contents/MacOS/sextant mcp
```

## Build from source

You need Rust (stable), [Bun](https://bun.sh) and Xcode command line tools.

```sh
bun install
scripts/fetch-onnxruntime.sh   # the speech runtime, bundled into the app
bun tauri dev                  # run with data in ./.dev-data
bun tauri build                # build Sextant.app and the DMG
scripts/stop-hook.sh           # format, lint, tests and the network check
```

Dev builds read and write only `./.dev-data`. See `CLAUDE.md` for all commands and `plan.md` for the design.

## Cut a release

You need a paid Apple Developer account. The `curl` installer also needs the repository to be public.

1. Create a "Developer ID Application" certificate, export it as a `.p12` file and encode it: `base64 -i cert.p12 | pbcopy`.
2. Add these repository secrets (Settings, Secrets and variables, Actions):

   | Secret                       | Value                                                                            |
   | ---------------------------- | -------------------------------------------------------------------------------- |
   | `APPLE_CERTIFICATE`          | The base64 text of the `.p12` file                                               |
   | `APPLE_CERTIFICATE_PASSWORD` | The password you set on export                                                   |
   | `APPLE_SIGNING_IDENTITY`     | The certificate name, for example `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID`                   | Your Apple ID email                                                              |
   | `APPLE_PASSWORD`             | An app-specific password from appleid.apple.com                                  |
   | `APPLE_TEAM_ID`              | Your 10-character team ID                                                        |

3. Set the same version in `app/tauri.conf.json`, `package.json` and `Cargo.toml`, commit it, then tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
4. The `Release` workflow builds, signs with the hardened runtime, notarizes, staples and publishes `Sextant.dmg`, `Sextant.dmg.sha256` and `install.sh`.

To check a release, run the install command on a Mac that has never seen Sextant. It must open without a Gatekeeper warning, ask for each permission only when its onboarding step needs it, and show a map.

## Licenses

Sextant is MIT licensed. See `LICENSE`.

- **Parakeet TDT 0.6B v3** by NVIDIA, [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/), used as the ONNX export by istupakov. NVIDIA is the credited author.
- **Silero VAD** by the Silero team, MIT.
- **ONNX Runtime** by Microsoft, MIT, bundled in the app.
- The Rust and JavaScript dependencies keep their own licenses.
