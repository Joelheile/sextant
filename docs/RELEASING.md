# Cut a release

You need a paid Apple Developer account. The `curl` installer and the updater also need the repository to be public.

## Set up once

1. Create a "Developer ID Application" certificate in Xcode (Settings, Accounts, Manage Certificates) or at developer.apple.com. Export it from Keychain Access as a `.p12` file with a password, then encode it: `base64 -i cert.p12 | pbcopy`.
2. Create an app-specific password at account.apple.com (Sign-In and Security, App-Specific Passwords). Notarization signs in with it.
3. Make the update signing key: `bun tauri signer generate -w ~/.tauri/sextant.key`. Its public key is in `plugins.updater.pubkey` in `app/tauri.conf.json`. Keep the private key and its password in a password manager: without them, installed copies can never update again.
4. Add these repository secrets (Settings, Secrets and variables, Actions), or set each with `gh secret set NAME`:

   | Secret                               | Value                                                                            |
   | ------------------------------------ | -------------------------------------------------------------------------------- |
   | `APPLE_CERTIFICATE`                  | The base64 text of the `.p12` file                                               |
   | `APPLE_CERTIFICATE_PASSWORD`         | The password you set on export                                                   |
   | `APPLE_SIGNING_IDENTITY`             | The certificate name, for example `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID`                           | Your Apple ID email                                                              |
   | `APPLE_PASSWORD`                     | The app-specific password from step 2                                            |
   | `APPLE_TEAM_ID`                      | Your 10-character team ID                                                        |
   | `TAURI_SIGNING_PRIVATE_KEY`          | The contents of `~/.tauri/sextant.key`                                           |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The key's password                                                               |

Never change the team, the signing identity or the bundle identifier (`app.sextant.desktop`) after the first release. macOS ties the Screen Recording and Accessibility grants to them, so a change makes every user grant the permissions again.

## Each release

1. Set the same version in `app/tauri.conf.json`, `package.json` and `Cargo.toml` (the workflow fails if they differ from the tag), commit it, then tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
2. The `Release` workflow builds, signs with the hardened runtime, notarizes, staples and publishes `Sextant.dmg`, `Sextant.dmg.sha256`, `install.sh`, `Sextant.app.tar.gz` and `latest.json`.

## How updates reach users

The installed app checks `https://github.com/Joelheile/sextant/releases/latest/download/latest.json` at start and then once a day (`app/src/update.rs`). It wakes every hour and reads the wall clock, so a Mac that sleeps at night still checks daily, and a failed check runs again an hour later. When the version there is newer, it downloads `Sextant.app.tar.gz`, checks its signature against the public key, replaces the app and opens it again. When launchd started Sextant, it exits with code 75 instead and launchd opens the new app, because launchd kills any process the old one spawned. The new app is signed by the same team, so the permissions stay granted. Dev builds and cargo builds never update.

The install script also updates: running it again stops Sextant and its launch agent, swaps in the new app and starts it through the launch agent. It refuses an app that fails `codesign --verify` or `spctl --assess`.

## Check a release

Run the install command on a Mac that has never seen Sextant. It must open without a Gatekeeper warning, open the panel with the setup steps, ask for each permission only when setup reaches it, and `sextant status` must show capture running.

To check the updater, install the previous release, publish the new one, then quit and open Sextant. Within a minute it opens again, `defaults read /Applications/Sextant.app/Contents/Info.plist CFBundleShortVersionString` prints the new version, and `sextant status` shows capture still running.

## While the repository is private

The `curl` command and the updater can't download anything. Use the GitHub CLI instead: `gh release download -R Joelheile/sextant -p Sextant.dmg`, then open the disk image and drag the app to `/Applications`.
