#!/bin/sh
# Installs the latest Sextant release:
#   curl -fsSL https://github.com/Joelheile/sextant/releases/latest/download/install.sh | sh
# It downloads the DMG over HTTPS, checks its SHA-256 against the checksum published with the
# release, copies Sextant.app into /Applications and opens it.
set -eu

repo="Joelheile/sextant"
base="https://github.com/$repo/releases/latest/download"
dest="${SEXTANT_INSTALL_DIR:-/Applications}"

fail() {
  echo "sextant install: $*" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "Sextant runs on macOS only."
[ "$(uname -m)" = "arm64" ] || fail "Sextant needs a Mac with Apple silicon."
supported=$(sw_vers -productVersion | awk -F . '{ print ($1 > 14 || ($1 == 14 && $2 >= 2)) }')
[ "$supported" = 1 ] || fail "Sextant needs macOS 14.2 or later."

work=$(mktemp -d)
mount="$work/mount"
cleanup() {
  hdiutil detach "$mount" -quiet 2>/dev/null || true
  rm -rf "$work"
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

rm -rf "$dest/Sextant.app"
ditto "$mount/Sextant.app" "$dest/Sextant.app"
echo "Installed $dest/Sextant.app"
open "$dest/Sextant.app"
