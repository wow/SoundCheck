#!/usr/bin/env bash
# Downloads the EBU loudness test set (Tech 3341/3342 signals) into tests/fixtures/ebu-loudness-test-set/.
# The files are large and not committed; tests that need them are #[ignore]d unless SC_EBU_TESTSET=1.
set -euo pipefail
cd "$(dirname "$0")/.."
DEST="tests/fixtures/ebu-loudness-test-set"
URL="https://tech.ebu.ch/files/live/sites/tech/files/shared/testmaterial/ebu-loudness-test-setv05.zip"
if [ -f "$DEST/.complete" ]; then echo "EBU test set already present in $DEST"; exit 0; fi
mkdir -p "$DEST"
echo "downloading $URL"
curl -fsSL -o "$DEST/ebu-loudness-test-set.zip" "$URL"
(cd "$DEST" && unzip -q -o ebu-loudness-test-set.zip && rm ebu-loudness-test-set.zip && touch .complete)
echo "EBU test set ready: $(find "$DEST" -name '*.wav' | wc -l | tr -d ' ') wav files"
