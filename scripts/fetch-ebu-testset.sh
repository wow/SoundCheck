#!/usr/bin/env bash
# Fetches the EBU loudness test set (Tech 3341/3342 signals) into tests/fixtures/ebu-loudness-test-set/.
# The files are large and not committed; tests that need them are #[ignore]d unless SC_EBU_TESTSET=1.
#
# tech.ebu.ch refuses non-browser downloads (HTTP 403), so this script also accepts a zip you
# downloaded by hand from https://tech.ebu.ch/publications/ebu_loudness_test_set and dropped into
# the destination directory, or a mirror URL in EBU_TESTSET_URL.
set -euo pipefail
cd "$(dirname "$0")/.."
DEST="tests/fixtures/ebu-loudness-test-set"
ZIP="$DEST/ebu-loudness-test-set.zip"
URL="${EBU_TESTSET_URL:-https://tech.ebu.ch/files/live/sites/tech/files/shared/testmaterial/ebu-loudness-test-setv05.zip}"
if [ -f "$DEST/.complete" ]; then echo "EBU test set already present in $DEST"; exit 0; fi
mkdir -p "$DEST"
if [ ! -f "$ZIP" ]; then
  echo "downloading $URL"
  if ! curl -fsSL -o "$ZIP" "$URL"; then
    rm -f "$ZIP"
    cat >&2 <<MSG
could not download the EBU loudness test set ($URL).
The EBU site blocks automated downloads. Download 'ebu-loudness-test-setv05.zip' in a browser from
  https://tech.ebu.ch/publications/ebu_loudness_test_set
save it as $ZIP and run this script again (or set EBU_TESTSET_URL to a mirror).
MSG
    exit 1
  fi
fi
(cd "$DEST" && unzip -q -o "$(basename "$ZIP")" && rm "$(basename "$ZIP")" && touch .complete)
echo "EBU test set ready: $(find "$DEST" -name '*.wav' | wc -l | tr -d ' ') wav files"
