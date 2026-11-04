#!/usr/bin/env bash
# Fetches a small speech subset for `sextant bench --asr` into fixtures/speech (git-ignored):
#   en/  LibriSpeech test-clean (CC BY 4.0), the first 8 MiB of the archive
#   de/  Multilingual LibriSpeech German test (CC BY 4.0), one shard
# Each clip becomes <id>.wav (16 kHz mono 16 bit) next to <id>.txt with the reference text.
# Every download is pinned by URL and SHA-256. Needs curl, tar and afconvert (macOS).
set -euo pipefail
cd "$(dirname "$0")/.."

CLIPS=40
out=fixtures/speech

LIBRI_URL=https://www.openslr.org/resources/12/test-clean.tar.gz
LIBRI_BYTES=8388608
LIBRI_SHA=df72fd5c4132a520eba9abadec9b58ab5466bd3c5f8272942326dddc9661acdf

MLS_COMMIT=2e83e61823b4c47dcbcb1980bb88601274127609
MLS_BASE=https://huggingface.co/datasets/facebook/multilingual_librispeech/resolve/$MLS_COMMIT/data/mls_german/test
MLS_SHARD=1054_869_000
MLS_SHARD_SHA=2518e7ac3ad51da1a854faa901f037a5060f02f3291fc1ff26a34b91975f183a
MLS_TRANSCRIPTS_SHA=767437e5873ab8403a7b89ee1e44443964c3688234d4145b6542471e7f58c628

if [ -d "$out/en" ] && [ -d "$out/de" ]; then
  echo "fixtures already in $out"
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

verify() {
  local got
  got=$(shasum -a 256 "$1" | awk '{print $1}')
  if [ "$got" != "$2" ]; then
    echo "checksum mismatch for $1: got $got" >&2
    exit 1
  fi
}

convert() {
  afconvert -f WAVE -d LEI16@16000 -c 1 "$1" "$2"
}

mkdir -p "$work/en" "$work/de" "$out"

echo "LibriSpeech test-clean (first $LIBRI_BYTES bytes)"
curl --proto '=https' --tlsv1.2 -fsSL -r "0-$((LIBRI_BYTES - 1))" -o "$work/libri.part" "$LIBRI_URL"
verify "$work/libri.part" "$LIBRI_SHA"
# The archive is cut mid-file: tar fails at the end, and the last file it lists is incomplete.
mkdir "$work/libri"
tar -xzf "$work/libri.part" -C "$work/libri" 2>/dev/null || true
cut=$(tar -tzf "$work/libri.part" 2>/dev/null | grep '\.flac$' | tail -n 1 || true)
rm -f "$work/libri/$cut"
count=0
for flac in $(find "$work/libri" -name '*.flac' | sort); do
  id=$(basename "$flac" .flac)
  line=$(grep -h "^$id " "$(dirname "$flac")"/*.trans.txt | head -n 1 || true)
  [ -n "$line" ] || continue
  convert "$flac" "$work/en/$id.wav"
  printf '%s\n' "${line#"$id "}" > "$work/en/$id.txt"
  count=$((count + 1))
  [ "$count" -lt "$CLIPS" ] || break
done
echo "  $count clips"

echo "Multilingual LibriSpeech German test, shard $MLS_SHARD"
curl --proto '=https' --tlsv1.2 -fsSL -o "$work/mls.tar.gz" "$MLS_BASE/audio/$MLS_SHARD.tar.gz"
verify "$work/mls.tar.gz" "$MLS_SHARD_SHA"
curl --proto '=https' --tlsv1.2 -fsSL -o "$work/mls.txt" "$MLS_BASE/transcripts.txt"
verify "$work/mls.txt" "$MLS_TRANSCRIPTS_SHA"
mkdir "$work/mls"
tar -xzf "$work/mls.tar.gz" -C "$work/mls"
count=0
for flac in $(find "$work/mls" -name '*.flac' | sort); do
  id=$(basename "$flac" .flac)
  text=$(awk -F'\t' -v id="$id" '$1 == id { print $2; exit }' "$work/mls.txt")
  [ -n "$text" ] || continue
  convert "$flac" "$work/de/$id.wav"
  printf '%s\n' "$text" > "$work/de/$id.txt"
  count=$((count + 1))
  [ "$count" -lt "$CLIPS" ] || break
done
echo "  $count clips"

rm -rf "$out/en" "$out/de"
mv "$work/en" "$work/de" "$out/"
echo "fixtures are in $out"
