#!/usr/bin/env bash
# Fetches the ONNX Runtime library (MIT, arm64) into app/frameworks (git-ignored). `tauri build`
# bundles it into Sextant.app/Contents/Frameworks and signs it with the app, so the hardened
# runtime loads it. Dev builds find it through ORT_DYLIB_PATH in .cargo/config.toml.
# The download is pinned by URL and SHA-256. Needs curl, tar and shasum (macOS).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=1.31.1
URL=https://github.com/microsoft/onnxruntime/releases/download/v$VERSION/onnxruntime-osx-arm64-$VERSION.tgz
ARCHIVE_SHA=2e1344a1926ee50eb25b38153cb6992c3567515aef0e677a1a520a1798896d6f
MEMBER=onnxruntime-osx-arm64-$VERSION/lib/libonnxruntime.$VERSION.dylib
MEMBER_SHA=17500d6b1e37e70f43a2c57d94233a1bc29fbcb297be0a5f8c7c13184f3307ba

out=app/frameworks/libonnxruntime.dylib

check() {
  echo "$2  $1" | shasum -a 256 -c - >/dev/null 2>&1
}

if [ -f "$out" ] && check "$out" "$MEMBER_SHA"; then
  echo "$out is up to date"
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL --proto '=https' -o "$work/onnxruntime.tgz" "$URL"
check "$work/onnxruntime.tgz" "$ARCHIVE_SHA" || { echo "ONNX Runtime archive does not match its checksum" >&2; exit 1; }
tar -xzf "$work/onnxruntime.tgz" -C "$work" "$MEMBER"
check "$work/$MEMBER" "$MEMBER_SHA" || { echo "ONNX Runtime library does not match its checksum" >&2; exit 1; }

mkdir -p "$(dirname "$out")"
mv "$work/$MEMBER" "$out"
echo "ONNX Runtime $VERSION is in $out"
