#!/usr/bin/env bash
# Fetches the "Beat This!" model files into models/ (gitignored) for development and CI.
#
# The small model and the mel front end are pinned to one commit of the Rust port's repository
# and verified by SHA-256; the full model (developer option, 83 MB) comes from the port's
# "model-large" release with the checksum it publishes. Weights: MIT (CPJKU/beat_this,
# converted to ONNX by danigb/beat-this-rs).
#
# Usage: scripts/fetch-models.sh [--full] [DEST]   (DEST defaults to models/)
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="danigb/beat-this-rs"
COMMIT="089b509247e6fdcec666511c0dcf0d5f39c21e73"
FULL_TAG="model-large"

FULL=0
DEST="models"
for arg in "$@"; do
  case "$arg" in
    --full) FULL=1 ;;
    -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
    *) DEST="$arg" ;;
  esac
done
mkdir -p "$DEST"

fetch_pinned() {
  local name="$1" sha="$2" path="$DEST/$1"
  if [ -f "$path" ] && echo "$sha  $path" | shasum -a 256 -c --status; then
    echo "present  $path"
    return
  fi
  echo "fetching $name"
  curl -fsSL --retry 3 -o "$path.part" "https://raw.githubusercontent.com/$REPO/$COMMIT/models/$name"
  echo "$sha  $path.part" | shasum -a 256 -c --status || { rm -f "$path.part"; echo "checksum mismatch for $name" >&2; exit 1; }
  mv "$path.part" "$path"
  echo "verified $path"
}

fetch_pinned mel_spectrogram.onnx fdd59e65c515331308e4c8841edf99972deca646bdf6197744c2a5b7755e3de9
fetch_pinned beat_this_small.onnx a5f8d39d989f31859454ba27afe61c5317ca95e4d9373e6853e5361b8937172f

if [ "$FULL" = 1 ]; then
  name="beat_this.onnx"; path="$DEST/$name"
  if [ ! -f "$path" ]; then
    echo "fetching $name (full model, about 83 MB)"
    base="https://github.com/$REPO/releases/download/$FULL_TAG"
    curl -fsSL --retry 3 -o "$path.part" "$base/$name"
    curl -fsSL --retry 3 -o "$path.sha256" "$base/$name.sha256"
    expected="$(cut -d' ' -f1 "$path.sha256")"
    echo "$expected  $path.part" | shasum -a 256 -c --status || { rm -f "$path.part"; echo "checksum mismatch for $name" >&2; exit 1; }
    mv "$path.part" "$path"; rm -f "$path.sha256"
    echo "verified $path"
  else
    echo "present  $path"
  fi
fi
echo "models ready in $DEST"
