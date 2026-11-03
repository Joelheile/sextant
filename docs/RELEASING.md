# Cut a release

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

To check a release, run the install command on a Mac that has never seen Sextant. It must open without a Gatekeeper warning, open `sextant setup` in Terminal, ask for each permission only when setup reaches it, and `sextant status` must show capture running.

## While the repository is private

The `curl` command can't download anything. Use the GitHub CLI instead: `gh release download -R Joelheile/sextant -p Sextant.dmg`, then open the disk image and drag the app to `/Applications`.
