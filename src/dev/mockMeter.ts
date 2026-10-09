/**
 * The mock player's meter readings: a true peak that jumps on each kick and decays over 90 ms,
 * reaching the row's analysed true peak on the kicks, and a momentary loudness that drifts around
 * the row's S-P95, as the engine reports them (OUT is IN plus the planned gain; no loudness in
 * the first 400 ms after a start or seek). Never bundled into the app.
 */
import type { MeterFrame, RowAnalysis } from '@/lib/ipc';

/** What one reading covers: the frames since the one before (30 a second). */
const SPAN_S = 1 / 30;

function round(db: number): number {
  return Math.round(db * 1000) / 1000;
}

/** The last kick at or before `at` (samples), or -1. */
function lastKick(kicks: Float64Array, at: number): number {
  let lo = 0;
  let hi = kicks.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((kicks[mid] ?? Infinity) <= at) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

export function mockMeterFrame(
  position: number,
  /** Frames played since the last start or seek. */
  played: number,
  rate: number,
  kicks: Float64Array,
  analysis: RowAnalysis,
  gain: number,
): MeterFrame {
  const k = lastKick(kicks, position);
  const kick = k >= 0 ? (kicks[k] ?? 0) : -Infinity;
  const since = (position - kick) / rate;
  // The loudest moment since the reading before: a kick within it reads at its full peak.
  const env = since <= SPAN_S ? 1 : 0.22 + 0.7 * Math.exp(-(since - SPAN_S) / 0.09);
  const bed = k < 0 ? 0.12 : env;
  const wobble = 0.4 * Math.sin(position / 7919);
  const inPeak = round(analysis.truePeak + 20 * Math.log10(bed) - Math.abs(wobble));
  const t = position / rate;
  const centre = (analysis.shortTermP95 ?? -14) - 1.2;
  const momentary = k < 0 ? centre - 14 : centre + 1.6 * Math.sin(t / 5) + 0.8 * Math.sin(t * 1.7);
  const inMomentary = played < 0.4 * rate ? null : round(momentary);
  return {
    position: Math.round(position),
    inPeak,
    inMomentary,
    outPeak: round(inPeak + gain),
    outMomentary: inMomentary === null ? null : round(inMomentary + gain),
    folded: false,
  };
}
