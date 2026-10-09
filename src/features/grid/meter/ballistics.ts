import type { MeterFrame } from '@/lib/ipc';

/**
 * How the meter strips move between the engine's readings (at most 30 a second), as pure
 * functions of time in milliseconds (`performance.now`), so the behaviour is tested without a
 * canvas.
 *
 * The bar is the true peak with an instant attack that falls 20 dB in 1.7 s, the return time of
 * an IEC 60268-10 type I programme meter. The peak-hold tick holds the highest peak for 1.5 s,
 * then falls at the same rate. The momentary loudness tick follows its readings through a
 * 100 ms one-pole smoother.
 */

/** How long the peak-hold tick stays put. */
export const HOLD_MS = 1500;
/** How fast the bar and the released hold tick fall: 20 dB per 1.7 s. */
export const FALL_DB_PER_MS = 20 / 1700;
/** Time constant of the loudness tick's smoother. */
export const SMOOTH_MS = 100;

/** A level set at a time: what a falling level started from, and when. */
export interface Mark {
  /** dB; `-Infinity` for nothing. */
  level: number;
  at: number;
}

export interface Peak {
  bar: Mark;
  hold: Mark;
}

export const NO_PEAK: Peak = {
  bar: { level: -Infinity, at: 0 },
  hold: { level: -Infinity, at: 0 },
};

/** The bar at `now`: the last peak, fallen since. */
export function barLevel(p: Peak, now: number): number {
  return p.bar.level - FALL_DB_PER_MS * Math.max(0, now - p.bar.at);
}

/** The peak-hold tick at `now`: the held peak, fallen since its hold ran out. */
export function holdLevel(p: Peak, now: number): number {
  return p.hold.level - FALL_DB_PER_MS * Math.max(0, now - p.hold.at - HOLD_MS);
}

/**
 * The peak after a reading of `db` (null: digital silence) at `now`. A reading at or above the
 * bar takes it at once; one at or above the hold tick holds it again for 1.5 s.
 */
export function peakInput(p: Peak, db: number | null, now: number): Peak {
  if (db === null) return p;
  const bar = db >= barLevel(p, now) ? { level: db, at: now } : p.bar;
  const hold = db >= holdLevel(p, now) ? { level: db, at: now } : p.hold;
  return bar === p.bar && hold === p.hold ? p : { bar, hold };
}

/**
 * The loudness tick `dt` ms after it showed `shown`, heading for `target`. No reading (silence,
 * or the first 400 ms after a start or seek) hides it; the first reading after that shows at
 * once rather than sweeping up from the bottom.
 */
export function smoothed(shown: number | null, target: number | null, dt: number): number | null {
  if (target === null) return null;
  if (shown === null || !Number.isFinite(shown)) return target;
  return shown + (target - shown) * (1 - Math.exp(-Math.max(0, dt) / SMOOTH_MS));
}

/** Which meter went over its limit; it stays so until cleared or a new track opens. */
export interface Overs {
  in: boolean;
  out: boolean;
}

export const NO_OVERS: Overs = { in: false, out: false };

/**
 * The overs after `frame`: IN over 0 dBTP, OUT over the true-peak `ceiling`. Latched: a reading
 * back under the limit does not clear them. The same object when nothing changed.
 */
export function latched(overs: Overs, frame: MeterFrame | null, ceiling: number): Overs {
  if (!frame) return overs;
  const inOver = overs.in || (frame.inPeak !== null && frame.inPeak > 0);
  const outOver = overs.out || (frame.outPeak !== null && frame.outPeak > ceiling);
  return inOver === overs.in && outOver === overs.out ? overs : { in: inOver, out: outOver };
}
