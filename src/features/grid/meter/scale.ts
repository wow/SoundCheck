import type { LoudnessMode, RowAnalysis } from '@/lib/ipc';
import { level, peak } from '@/lib/format';

/**
 * The meter strips' scale and words: one axis for true peak (dBTP) and loudness (LUFS), from
 * -36 to +3 dB, and the captions shown while the player is stopped.
 */

export const SCALE_MIN = -36;
export const SCALE_MAX = 3;
/** The labelled ticks, dB. */
export const TICKS = [0, -6, -12, -18, -24, -30] as const;

/** The y of `db` in a bar running from `top` (+3 dB) to `bottom` (-36 dB), clamped to it. */
export function levelY(db: number, top: number, bottom: number): number {
  const clamped = Math.min(SCALE_MAX, Math.max(SCALE_MIN, db));
  return top + ((SCALE_MAX - clamped) / (SCALE_MAX - SCALE_MIN)) * (bottom - top);
}

/**
 * The bar's fixed colour zones, as on an LED meter: green below -12 dB, yellow from -12 to
 * -3 dB, red above -3 dB. A bar at -2 dB is green at the bottom, yellow, then red at the top.
 */
export type Zone = 'ok' | 'warn' | 'err';
export const WARN_FROM = -12;
export const ERR_FROM = -3;

/** The zone a level falls in. */
export function zoneOf(db: number): Zone {
  return db > ERR_FROM ? 'err' : db > WARN_FROM ? 'warn' : 'ok';
}

/** The bar's runs from the bottom of the scale up to `db`: each zone's span it reaches. */
export function zoneRuns(db: number): { zone: Zone; from: number; to: number }[] {
  const top = Math.min(SCALE_MAX, db);
  const runs: { zone: Zone; from: number; to: number }[] = [];
  const spans: [Zone, number, number][] = [
    ['ok', SCALE_MIN, WARN_FROM],
    ['warn', WARN_FROM, ERR_FROM],
    ['err', ERR_FROM, SCALE_MAX],
  ];
  for (const [zone, from, to] of spans) {
    if (top <= from) break;
    runs.push({ zone, from, to: Math.min(to, top) });
  }
  return runs;
}

export type Side = 'in' | 'out';

/** What one strip shows while stopped: the track's numbers, or the planned ones for OUT. */
export interface Caption {
  /** `S-P95` in DJ mode, `Integrated` in Streaming mode. */
  stat: string;
  /** LUFS; null when the analysis has none (silence). */
  loudness: number | null;
  /** dBTP. */
  truePeak: number | null;
}

/**
 * The stopped captions: IN is the analysis, OUT the same plus the planned gain (OUT reads IN
 * plus the gain exactly while playing, too).
 */
export function caption(
  side: Side,
  analysis: RowAnalysis | undefined,
  mode: LoudnessMode,
  gain: number,
): Caption {
  const stat = mode === 'dj' ? 'S-P95' : 'Integrated';
  if (!analysis) return { stat, loudness: null, truePeak: null };
  const measured = mode === 'dj' ? analysis.shortTermP95 : analysis.integrated;
  const offset = side === 'out' ? gain : 0;
  return {
    stat,
    loudness: measured === null ? null : measured + offset,
    truePeak: analysis.truePeak + offset,
  };
}

/** `S-P95 -9.2 · TP -0.3`, as the strip writes it. */
export function captionText(c: Caption): string {
  return `${c.stat} ${level(c.loudness)} · TP ${peak(c.truePeak)}`;
}

/** A live reading as the readout writes it; `–` for none. */
export function readingText(db: number | null, isPeak: boolean): string {
  if (db === null || !Number.isFinite(db)) return '–';
  return isPeak ? peak(db) : level(db);
}

export const SIDE_LABEL: Record<Side, string> = { in: 'IN', out: 'OUT' };
