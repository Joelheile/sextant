# Contributing

## Build from source

You need Rust (stable), [Bun](https://bun.sh) and Xcode command line tools.

```sh
bun install                    # the Tauri CLI, for bundling
scripts/fetch-onnxruntime.sh   # the speech runtime, bundled into the app
cargo run                      # run with data in ./.dev-data
cargo run -q -- tui            # the terminal UI for that running app
bun tauri build                # build Sextant.app and the DMG
scripts/stop-hook.sh           # format, lint, tests and the network check
```

Dev builds read and write only `./.dev-data`. See `CLAUDE.md` for all commands and `plan.md` for the design.

## Before you open a pull request

- Run `scripts/stop-hook.sh`. It must pass.
- Add a test for every bug fix and feature.
- Keep network code in `app/src/models.rs`. `scripts/check-network.sh` fails the build anywhere else.
- Never commit real screen captures, audio or personal data. Only test fixtures may hold images or audio.

## The README animation

`docs/assets/demo.gif` comes from the Remotion project in `video/`. It uses sample data.

```sh
cd video
bun install
bun run dev                    # preview in Remotion Studio
bunx remotion render Hero ../docs/assets/demo.gif --codec=gif --every-nth-frame=2 --scale=0.8
```

## Cut a release

See [docs/RELEASING.md](docs/RELEASING.md).
