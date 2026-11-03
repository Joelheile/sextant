# Privacy

- Your data stays on your Mac, in one encrypted database, until an agent you connect reads it. The key lives in your Keychain.
- Sextant never stores pixels or audio. Screen text and transcripts are stored as text only.
- It reads only the focused window, and only when no gate is closed: excluded apps (password managers and messengers by default), excluded sites (banks and payment sites by default, with their subdomains), private browser windows, secure input, paused, or the source switched off. Excluded sites are also dropped from browser history. Call transcription follows the same gates: it does not start for an excluded app or a private browser window.
- It notes which labelled buttons you click and the text left in a field when you move on. It never records keystrokes and never reads secure (password) fields.
- Emails, IBANs, card numbers, API keys and tokens are redacted before anything is stored.
- Sextant runs no language model of its own. Your agent answers questions by calling Sextant's search tools, and each answer it gets is listed in the sent log.
- The only network call is the speech model download you start yourself, from pinned URLs with a SHA-256 check. There is no telemetry, crash reporter, analytics or auto-updater.
- In Low Power Mode, Sextant takes no screenshots and runs no OCR.
- You choose how long raw data stays: 30, 90 or 365 days, or forever, and how much space it may use: 1, 2, 5 or 10 GB, or no limit. Past the limit, the oldest days go first. "Delete recent" deletes the last 15 minutes, hour or day. "Delete everything" empties the data folder. "Uninstall" does that, removes Sextant from Claude Desktop and moves the app to the Trash.
- If you tell your agent to read from Sextant, the text it reads goes to wherever that agent's model runs, which may be the cloud. Excluded apps and redaction apply to that output too. "Sent to your agent" in Privacy lists each answer an agent got, with the time, the client, the tool and the size.

Transcribing people without their consent is a crime in some countries, including Germany (section 201 StGB). Tell the other side before you transcribe a call.
