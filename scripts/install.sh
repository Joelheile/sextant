#!/bin/sh
# Installs or updates to the latest Sextant release:
#   curl -fsSL https://github.com/Joelheile/sextant/releases/latest/download/install.sh | sh
# It downloads the DMG over HTTPS, checks its SHA-256 against the checksum published with the
# release, checks that Apple notarized the app inside, stops a running Sextant, swaps the app
# into /Applications, links the sextant command into /usr/local/bin when that folder is
# writable, and starts the app.
set -eu

repo="Joelheile/sextant"
base="https://github.com/$repo/releases/latest/download"
dest="${SEXTANT_INSTALL_DIR:-/Applications}"
label="app.sextant.desktop"
agent="$HOME/Library/LaunchAgents/$label.plist"
domain="gui/$(id -u)"

fail() {
  echo "sextant install: $*" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "Sextant runs on macOS only."
[ "$(uname -m)" = "arm64" ] || fail "Sextant needs a Mac with Apple silicon."
major=$(sw_vers -productVersion | cut -d . -f 1)
[ "$major" -ge 26 ] || fail "Sextant needs macOS 26 or later."
[ -d "$dest" ] && [ -w "$dest" ] || fail "can't write to $dest. Install into your own folder instead:
  curl -fsSL $base/install.sh | SEXTANT_INSTALL_DIR=~/Applications sh"

work=$(mktemp -d)
mount="$work/mount"
staged="$dest/.Sextant.app.new"
cleanup() {
  hdiutil detach "$mount" -quiet 2>/dev/null || true
  rm -rf "$work" "$staged"
}
trap cleanup EXIT

echo "Downloading Sextant..."
curl -fsSL --proto '=https' --tlsv1.2 -o "$work/Sextant.dmg" "$base/Sextant.dmg"
curl -fsSL --proto '=https' --tlsv1.2 -o "$work/Sextant.dmg.sha256" "$base/Sextant.dmg.sha256"

expected=$(cut -d ' ' -f 1 "$work/Sextant.dmg.sha256")
actual=$(shasum -a 256 "$work/Sextant.dmg" | cut -d ' ' -f 1)
[ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "the download does not match its SHA-256. Nothing was installed."

mkdir "$mount"
hdiutil attach "$work/Sextant.dmg" -quiet -nobrowse -readonly -mountpoint "$mount"
[ -d "$mount/Sextant.app" ] || fail "the disk image has no Sextant.app."
if ! codesign --verify --deep --strict "$mount/Sextant.app" 2>/dev/null ||
  ! spctl --assess --type execute "$mount/Sextant.app" 2>/dev/null; then
  fail "the app is not signed and notarized. Nothing was installed."
fi

# Copy next to the old app first, so a failed copy leaves the old one in place.
rm -rf "$staged"
ditto "$mount/Sextant.app" "$staged"

# Stop a running Sextant. The launch agent goes first, or launchd would start the old app again.
bin="$dest/Sextant.app/Contents/MacOS/sextant"
launchctl bootout "$domain/$label" 2>/dev/null || true
pkill -f "^$bin" 2>/dev/null || true
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -f "^$bin" >/dev/null || break
  sleep 0.5
done

rm -rf "$dest/Sextant.app"
mv "$staged" "$dest/Sextant.app"
echo "Installed $dest/Sextant.app"

links="${SEXTANT_LINK_DIR:-/usr/local/bin}"
if [ -d "$links" ] && [ -w "$links" ] && ln -sf "$bin" "$links/sextant"; then
  echo "Linked the sextant command into $links"
else
  echo "To use the sextant command, run: sudo ln -sf '$bin' $links/sextant"
fi

# An update starts through the launch agent, so launchd restarts Sextant after a crash right
# away. A first install opens the app, which writes the agent for the next login.
if [ -f "$agent" ] && grep -q "<string>$bin</string>" "$agent"; then
  launchctl bootstrap "$domain" "$agent"
else
  open "$dest/Sextant.app"
fi
