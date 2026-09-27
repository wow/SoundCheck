/**
 * Tap tempo: the median interval of the last eight taps, from the fourth tap on. A pause of more
 * than two seconds starts a new count.
 */

const MAX_GAP_MS = 2000;
const KEEP = 8;
const MIN_TAPS = 4;

/** The taps so far after a tap at `now` (ms). */
export function tapped(taps: number[], now: number): number[] {
  const last = taps[taps.length - 1];
  const kept = last !== undefined && now - last <= MAX_GAP_MS ? taps : [];
  return [...kept, now].slice(-KEEP);
}

/** The tempo the taps give, in BPM, or null before the fourth tap. */
export function tapBpm(taps: number[]): number | null {
  if (taps.length < MIN_TAPS) return null;
  const intervals = taps
    .slice(1)
    .map((t, i) => t - (taps[i] ?? t))
    .sort((a, b) => a - b);
  const mid = intervals.length / 2;
  const median =
    intervals.length % 2 === 1
      ? intervals[Math.floor(mid)]!
      : (intervals[mid - 1]! + intervals[mid]!) / 2;
  return median > 0 ? 60_000 / median : null;
}
