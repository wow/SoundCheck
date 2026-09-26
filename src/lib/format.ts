import type {
  Codec,
  DjUnsafe,
  FileInfo,
  Plan,
  Reason,
  ReviewReason,
  RowAnalysis,
} from '@/lib/ipc';

/** The typographic minus the table uses for negative dB and LU. */
const MINUS = '−';

/** `+3.2` / `−3.2` with `digits` decimals. */
export function signed(value: number, digits = 1): string {
  const text = Math.abs(value).toFixed(digits);
  if (Number(text) === 0) return text;
  return (value < 0 ? MINUS : '+') + text;
}

/** A LUFS or dBTP value as the table shows it: `-7.8` (hyphen, tabular). */
export function level(value: number | null | undefined, digits = 1): string {
  return value == null ? '—' : value.toFixed(digits);
}

export const CODEC_LABEL: Record<Codec, string> = {
  wav: 'WAV',
  aiff: 'AIFF',
  flac: 'FLAC',
  mp3: 'MP3',
  aac: 'AAC',
  alac: 'ALAC',
  vorbis: 'OGG',
  opus: 'OPUS',
  other: '?',
};

/** `44.1k · 16`, `48k · 32f`, `320 kbps`, or `—` when the headers did not say. */
export function specText(info: FileInfo): string {
  if (info.bitrateKbps != null) return `${info.bitrateKbps} kbps`;
  if (info.sampleRate == null) return '—';
  const rate = `${Number((info.sampleRate / 1000).toFixed(1))}k`;
  if (info.bitsPerSample == null) return rate;
  return `${rate} · ${info.bitsPerSample}${info.float ? 'f' : ''}`;
}

export const DJ_UNSAFE_TEXT: Record<DjUnsafe, string> = {
  sampleRate: 'Not 44.1 or 48 kHz; export writes 44.1 or 48 kHz',
  bitDepth: 'Not 16- or 24-bit; export writes 24-bit PCM',
  float: 'Floating-point samples; export writes 24-bit PCM',
  channels: 'Not stereo; export writes stereo',
};

/** The title, or the file name without its extension. */
export function displayTitle(info: FileInfo, path: string): string {
  if (info.title) return info.title;
  const name = path.split('/').pop() ?? path;
  return name.replace(/\.[^.]+$/, '');
}

/** Minutes and seconds, `2:40`. */
export function clock(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

/** The Action column: what processing would do, and the detail under it. */
export function actionText(plan: Plan): {
  main: string;
  detail: string | null;
  tone: 'fg' | 'accent' | 'muted';
} {
  if (plan.skip) {
    const main =
      plan.skip.type === 'analyseOnly'
        ? `Skip: ${CODEC_LABEL[plan.skip.codec]} is analyse-only`
        : 'Skip: silent';
    const detail = plan.skip.type === 'analyseOnly' ? 'Convert to AIFF arrives with export' : null;
    return { main, detail, tone: 'muted' };
  }
  const gain = plan.gain;
  if (!gain || gain.type === 'atTarget') {
    return { main: 'Already at target', detail: null, tone: 'fg' };
  }
  if (gain.type === 'globalGain') {
    const main = gain.steps === 0 ? 'Global-gain ±0 dB' : `Global-gain ${signed(gain.gainDb)} dB`;
    return { main, detail: `Residual ${signed(gain.residualLu)} LU · MP3 moves in 1.5 dB steps`, tone: 'fg' };
  }
  if (gain.shortByLu > 0) {
    return {
      main: `Gain ${signed(gain.gainDb)} dB`,
      detail: `Short by ${gain.shortByLu.toFixed(1)} LU: the ceiling is reached`,
      tone: 'accent',
    };
  }
  return { main: `Gain ${signed(gain.gainDb)} dB`, detail: null, tone: 'fg' };
}

const REASON_TEXT: Record<Reason, string> = {
  residuals: 'beats off the grid',
  coverage: 'few beats on the grid',
  recall: 'missing beats',
  octaveMargin: 'check octave',
  downbeatMargin: 'check bar 1',
  meterMargin: 'meter to confirm',
  tagDisagrees: 'tag disagrees',
  outsideRange: 'outside your BPM range',
  short: 'too short to be sure',
  drifts: 'drifts',
  noKick: 'no kick to anchor',
};

/** One short phrase per review reason, as the Action column's second line shows them. */
export function reviewText(
  review: ReviewReason[],
  analysis: RowAnalysis | undefined,
  bpmRange: [number, number],
): string {
  const parts: string[] = [];
  for (const r of review) {
    switch (r.type) {
      case 'confidence': {
        const reasons = analysis?.grid?.reasons ?? [];
        const first = reasons.filter((x) => x !== 'drifts').map((x) => REASON_TEXT[x]);
        parts.push(first.length > 0 ? first.slice(0, 2).join(', ') : 'low confidence');
        break;
      }
      case 'drifts': {
        const g = analysis?.grid;
        parts.push(
          g
            ? `Drifts: max ${Math.round(g.residualMaxMs)} ms, ${signed(g.driftPpm, 0)} ppm`
            : 'Drifts',
        );
        break;
      }
      case 'outsideBpmRange':
        parts.push(`BPM outside ${bpmRange[0]}–${bpmRange[1]}`);
        break;
      case 'tagBpmDisagrees':
        parts.push(`tag says ${r.tag.toFixed(2)}`);
        break;
      case 'noGrid':
        parts.push('no beats found');
        break;
    }
  }
  return parts.join(' · ');
}
