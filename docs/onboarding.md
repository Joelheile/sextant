# Permission onboarding on macOS

Research from 2026-11-03 on how to get Sextant's permissions granted with the least friction, compared with what `sextant setup` does today. Apple publishes no guide for this, so most of it comes from Apple's API docs, Apple developer forum threads and apps that ship the same flow.

## What Sextant needs

| Permission                      | Used for                                      | Prompt API                                                         | Check API                             | Takes effect              |
| ------------------------------- | --------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------- | ------------------------- |
| Accessibility                   | Focused app, window, AX tree, presses, edits  | `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt` | `AXIsProcessTrusted`                  | Right away                |
| Screen & System Audio Recording | OCR fallback when the AX tree has little text | `CGRequestScreenCaptureAccess`                                     | `CGPreflightScreenCaptureAccess`      | After Sextant opens again |
| Automation, per browser         | The address of the browser tab                | `AEDeterminePermissionToAutomateTarget` with ask on                | The same call with ask off            | Right away                |
| Microphone                      | Your side of calls                            | `AVCaptureDevice.requestAccess(for: .audio)`, or the first stream  | `AVCaptureDevice.authorizationStatus` | Right away                |
| System audio (process tap)      | The other side of calls                       | The first start of an aggregate device with a tap                  | None public                           | Right away                |

Today's flow (`app/src/cli.rs`, `setup`): Screen Recording, then Accessibility, each by opening the pane with the drag panel and waiting for Enter. Then Automation through the system prompt, then a relaunch if Screen Recording was new. Audio is asked later, in `sextant audio on`.

## Findings

1. The system prompts add Sextant to the list, so the user flips a switch instead of dragging. `AXIsProcessTrustedWithOptions` with the prompt option and `CGRequestScreenCaptureAccess` both register the app in their Privacy & Security list and show an alert that opens the pane. Dragging the app in is only needed when it is missing from the list, for example after `tccutil reset` or when the app moved. Libraries that ship this flow ([PermissionFlow](https://swiftpackageregistry.com/jaywcjlove/PermissionFlow), [AskForPermission](https://github.com/riko2chen/AskForPermission)) still keep a floating guide next to System Settings, like Sextant's drag panel.
2. Each prompt shows once. After a denial, calling the prompt again does nothing, so the fallback is always the pane plus the guide. Accessibility can't tell "denied" from "never asked" ([car-desktop source](https://docs.rs/crate/car-desktop/latest/source/src/macos/permissions.rs)).
3. Polling works for Accessibility, not for Screen Recording. `AXIsProcessTrusted` turns true as soon as the switch flips, so the flow can move on without the user pressing anything. `CGPreflightScreenCaptureAccess` can stay false until the app opens again ([Apple forums 695689](https://developer.apple.com/forums/thread/695689)). Once the switch is on, macOS offers "Quit & Reopen" itself.
4. Screen Recording asks again every month. Since Sequoia, macOS shows a reminder with "Allow For One Month" for apps that record the screen, ScreenCaptureKit included ([9to5Mac](https://9to5mac.com/2024/08/14/macos-sequoia-screen-recording-prompt-monthly/), [TidBITS](https://tidbits.com/2024/08/19/apple-reduces-excessive-sequoia-permission-requests-shifts-to-monthly)). The only exemption, `com.apple.developer.persistent-content-capture`, is for VNC apps ([Michael Tsai](https://mjtsai.com/blog/2024/08/08/sequoia-screen-recording-prompts-and-the-persistent-content-capture-entitlement/)). I found no source that says macOS 26 changed this, so plan for it.
5. Grants follow the code signature. TCC (the macOS permission database) identifies an app by its designated requirement: team ID plus bundle ID for a Developer ID app. Ad hoc builds have no stable one, so every rebuild loses the grants. A change of team makes the old grants stale: they show as on but don't work, and only `tccutil reset` fixes them ([Apple forums 795739](https://developer.apple.com/forums/thread/795739)). The notarized build and its signed updates keep grants across versions.
6. macOS 26.1 lists only app bundles under Screen Recording. Plain executables no longer show up ([Apple forums 807898](https://developer.apple.com/forums/thread/807898)). Sextant asks from the app, never from the CLI, so this holds as long as the CLI keeps going through `sextant.sock`.
7. System audio has no public status check. The prompt shows on the first tap start. Without `NSAudioCaptureUsageDescription`, macOS denies silently and the buffers are all zeros ([Apple docs](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps), [AudioCap](https://github.com/insidegui/AudioCap)). Sextant has the key. The only way to notice a denial is silent buffers during a call.
8. Ask in context. Automation needs a running browser to ask about. Microphone and system audio make sense only when the user turns transcripts on. Sextant already defers audio. Automation is asked in setup, and is skipped when no browser runs.

## Recommendations

In order of value per line of code:

1. Accessibility first, through the system prompt, and move on by itself. Call `AXIsProcessTrustedWithOptions` with the prompt, open the drag panel only when Sextant is not in the list, and poll `AXIsProcessTrusted` every 500 ms instead of waiting for Enter. Capture already works with Accessibility alone, because the AX tree comes first.
2. Screen Recording last, and optional. Say what it adds ("text from apps that hide it from Accessibility, such as some PDFs and video calls"), call `CGRequestScreenCaptureAccess`, and let the user skip. Put it last because it needs the reopen. Write a "setup step" marker to the data folder so setup goes on after the reopen instead of starting over.
3. Automation when the browser first gets focus. Ask the first time a Chromium browser or Arc is focused after setup, once per browser, instead of during setup. The prompt then names the browser the user is looking at.
4. Detect lost grants and say which one. The tray already turns to the "!" ring through `capture_blockers`. Make its menu item open the step for the missing permission. That covers the monthly Screen Recording reminder and a revoked switch.
5. A stale entry needs a reset. When the switch is on but the check stays false after a reopen, offer to run `tccutil reset Accessibility app.sextant.desktop` (or `ScreenCapture`) and ask again.
6. Move setup out of Terminal (larger). The release goal is a first map without a terminal. A small native panel with one row per permission (status, one sentence on why, one button) would do it. The drag panel already proves AppKit panels work from Rust here. This is the biggest change, so decide it on its own.
7. Keep signing stable. Never change the team or `app.sextant.desktop` after the first release. For dev builds, signing with the Developer ID (or an Apple Development certificate) instead of ad hoc keeps grants between rebuilds.

## Copy for each step

One sentence on why, one on what is never stored, then the action:

- Accessibility: "Sextant reads the names of the app, window and buttons you use. It never reads passwords or keystrokes. Turn on Sextant in the list."
- Screen Recording: "When an app hides its text from Accessibility, Sextant reads it from the screen. The picture is never saved. Turn on Sextant, then click Quit & Reopen."
- Automation: "Sextant reads the address of the tab you are looking at, to group your work. Click Allow."
- Microphone and system audio: the `AUDIO_CONSENT` text in `app/src/cli.rs`, then the two system prompts on the first call.
