#!/usr/bin/env bash
# Integrated loudness of every audio file under a folder, measured by ffmpeg's ebur128 filter, as
# CSV on stdout: `file,ffmpeg_i_lufs`. The evaluation (`sc-cli eval`) compares SoundCheck's own
# integrated loudness against this column (target: within 0.1 LU).
#
# Usage: scripts/ffmpeg-reference.sh <folder> > reference.csv
# Requires ffmpeg on PATH. Files are named by their base name, NFC-normalised as SoundCheck does.
set -euo pipefail
dir="${1:?usage: scripts/ffmpeg-reference.sh <folder>}"
command -v ffmpeg >/dev/null || { echo "ffmpeg not found on PATH" >&2; exit 2; }
echo "file,ffmpeg_i_lufs"
find "$dir" -type f \( -iname '*.flac' -o -iname '*.wav' -o -iname '*.aif' -o -iname '*.aiff' -o -iname '*.mp3' -o -iname '*.m4a' \) -print0 |
  sort -z |
  while IFS= read -r -d '' f; do
    # -nostdin: ffmpeg would otherwise read the file list this loop is fed from. awk reads to the
    # end (no early exit) so ffmpeg never meets a closed pipe.
    i=$(ffmpeg -nostdin -hide_banner -nostats -i "$f" -map 0:a:0 -af ebur128=framelog=quiet -f null - 2>&1 |
      awk '/Integrated loudness:/ {section=1} section && /I:/ && !done {print $2; done=1}')
    name=$(basename "$f" | python3 -c 'import sys, unicodedata; print(unicodedata.normalize("NFC", sys.stdin.read().rstrip("\n")))')
    case "$name" in *,*|*\"*) name="\"${name//\"/\"\"}\"" ;; esac
    echo "$name,${i:-}"
  done
