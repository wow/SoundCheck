import type { Grid, Meter } from '@/lib/ipc';

/**
 * The grid view's arithmetic, kept pure so it is tested without a canvas: where a sample is on
 * screen, which grid lines fall in a range and how strongly each is marked, bar numbers, and the
 * waveform level to ask the engine for. Positions are samples at the file's rate throughout.
 */

/** What the canvas shows: the sample at its left edge and how many samples each CSS pixel spans. */
export interface View {
  start: number;
  samplesPerPx: number;
  widthPx: number;
}

export type LineKind = 'bar' | 'group' | 'pulse';

export interface GridLine {
  /** Index from bar 1's line (0); earlier lines are negative. */
  index: number;
  sample: number;
  kind: LineKind;
  /** Bar number: 1 from bar 1 on, -1, -2 ... for the pickup bars before it. */
  bar: number;
  /** Pulse within the bar, from 1. */
  pulse: number;
}

export function sampleToX(view: View, sample: number): number {
  return (sample - view.start) / view.samplesPerPx;
}

export function xToSample(view: View, x: number): number {
  return view.start + x * view.samplesPerPx;
}

/** Samples per beat of `grid` at `sampleRate`. */
export function samplesPerBeat(grid: Grid, sampleRate: number): number {
  return (60 * sampleRate) / grid.bpm;
}

export function pxPerBeat(view: View, grid: Grid, sampleRate: number): number {
  return samplesPerBeat(grid, sampleRate) / view.samplesPerPx;
}

/** The view zoomed by `factor` (> 1 zooms in) around CSS pixel `x`, kept within its limits. */
export function zoomAround(
  view: View,
  factor: number,
  x: number,
  minSpp: number,
  maxSpp: number,
): View {
  const at = xToSample(view, x);
  const samplesPerPx = Math.min(maxSpp, Math.max(minSpp, view.samplesPerPx / factor));
  return { ...view, samplesPerPx, start: at - x * samplesPerPx };
}

/** How strongly pulse `pulse` (0 = beat 1) of a bar of `meter` is marked, as the click does. */
export function lineKind(meter: Meter, pulse: number): LineKind {
  if (pulse === 0) return 'bar';
  let start = 0;
  for (const group of meter.grouping) {
    if (pulse === start) return 'group';
    start += group;
  }
  return 'pulse';
}

/**
 * The ruler's label for a pulse (from 1): its number at a group start, else its place in the
 * group, so 9/8 2+2+2+3 reads `1 2 3 2 5 2 7 2 3` and 4/4 reads `1 2 3 4`.
 */
export function rulerLabel(meter: Meter, pulse: number): string {
  let start = 1;
  for (const group of meter.grouping) {
    if (pulse < start + group) return String(pulse === start ? pulse : pulse - start + 1);
    start += group;
  }
  return String(pulse);
}

/**
 * Samples per pixel at the closest zoom (1 ms is 5 px) and the widest (the whole track across
 * `widthPx`).
 */
export function zoomLimits(sampleRate: number, frames: number, widthPx: number): [number, number] {
  const closest = sampleRate / 5000;
  return [closest, Math.max(closest, frames / Math.max(1, widthPx))];
}

/** The group (from 1) a pulse (from 1) falls in: pulse 5 of 9/8 2+2+2+3 is in group 3. */
export function groupOf(meter: Meter, pulse: number): number {
  let end = 0;
  for (const [g, size] of meter.grouping.entries()) {
    end += size;
    if (pulse <= end) return g + 1;
  }
  return meter.grouping.length;
}

export function barOf(index: number, beatsPerBar: number): number {
  const b = Math.floor(index / beatsPerBar);
  return b >= 0 ? b + 1 : b;
}

function mod(a: number, n: number): number {
  return ((a % n) + n) % n;
}

/** The grid's lines from sample `from` to `to`, rounded to samples as the click places them. */
export function linesInRange(grid: Grid, sampleRate: number, from: number, to: number): GridLine[] {
  const spb = samplesPerBeat(grid, sampleRate);
  if (!(spb > 0) || !Number.isFinite(spb)) return [];
  const bpb = Math.max(1, grid.meter.beatsPerBar);
  const lines: GridLine[] = [];
  for (let i = Math.ceil((from - 0.5 - grid.anchor) / spb); ; i++) {
    const sample = Math.round(grid.anchor + i * spb);
    if (sample > to) break;
    if (sample < from) continue;
    const pulse = mod(i, bpb);
    lines.push({
      index: i,
      sample,
      kind: lineKind(grid.meter, pulse),
      bar: barOf(i, bpb),
      pulse: pulse + 1,
    });
  }
  return lines;
}

/** The line nearest `sample`. */
export function nearestLine(grid: Grid, sampleRate: number, sample: number): GridLine {
  const spb = samplesPerBeat(grid, sampleRate);
  const i = Math.round((sample - grid.anchor) / spb);
  const bpb = Math.max(1, grid.meter.beatsPerBar);
  const pulse = mod(i, bpb);
  return {
    index: i,
    sample: Math.round(grid.anchor + i * spb),
    kind: lineKind(grid.meter, pulse),
    bar: barOf(i, bpb),
    pulse: pulse + 1,
  };
}

export type ResidualTone = 'ok' | 'warn' | 'err';

/** Colour class of a residual: within 10 ms, 25 ms, or beyond. */
export function residualTone(ms: number): ResidualTone {
  const a = Math.abs(ms);
  return a < 10 ? 'ok' : a < 25 ? 'warn' : 'err';
}

/**
 * The fit's P95 and max against the engine's verdict limits: static below 12 / 30 ms, static
 * with a warning below 25 / 50 ms, else Drifts.
 */
export function fitTones(p95Ms: number, maxMs: number): { p95: ResidualTone; max: ResidualTone } {
  const tone = (ms: number, ok: number, warn: number): ResidualTone =>
    ms < ok ? 'ok' : ms < warn ? 'warn' : 'err';
  return { p95: tone(p95Ms, 12, 25), max: tone(maxMs, 30, 50) };
}

/**
 * The largest |residual| of each bar from the per-line residuals (`firstLine` is the index of
 * `residuals[0]`); NaN for a bar without a matched attack.
 */
export function barResiduals(
  residuals: Float32Array,
  firstLine: number,
  beatsPerBar: number,
): Map<number, number> {
  const bars = new Map<number, number>();
  residuals.forEach((r, k) => {
    const bar = barOf(firstLine + k, beatsPerBar);
    const worst = bars.get(bar);
    if (Number.isNaN(r)) {
      if (worst === undefined) bars.set(bar, NaN);
    } else if (worst === undefined || Number.isNaN(worst) || Math.abs(r) > Math.abs(worst)) {
      bars.set(bar, r);
    }
  });
  return bars;
}

/**
 * The waveform level for a view: the largest power of two from 8 frames per bin that still
 * gives at least one bin per device pixel.
 */
export function samplesPerBin(samplesPerPx: number, devicePixelRatio: number): number {
  const perDevicePx = samplesPerPx / Math.max(1, devicePixelRatio);
  let level = 8;
  while (level * 2 <= perDevicePx) level *= 2;
  return level;
}

/** `0:34.902`: minutes, seconds and milliseconds of a sample position. */
export function clockText(sample: number, sampleRate: number): string {
  const ms = Math.max(0, Math.round((sample / sampleRate) * 1000));
  const m = Math.floor(ms / 60_000);
  const s = ((ms % 60_000) / 1000).toFixed(3).padStart(6, '0');
  return `${m}:${s}`;
}

/** How far a dragged bar 1 reaches for an attack to snap to, in seconds. */
export const SNAP_S = 0.04;

/** The nearest attack to `sample` within 40 ms, else `sample` itself. */
export function snapped(onsets: Float64Array, sample: number, sampleRate: number): number {
  const t = sample / sampleRate;
  let lo = 0;
  let hi = onsets.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((onsets[mid] ?? 0) < t) lo = mid + 1;
    else hi = mid;
  }
  let best: number | null = null;
  for (const i of [lo - 1, lo]) {
    const o = onsets[i];
    if (
      o !== undefined &&
      Math.abs(o - t) <= SNAP_S &&
      (best === null || Math.abs(o - t) < Math.abs(best - t))
    )
      best = o;
  }
  return best === null ? sample : Math.round(best * sampleRate);
}

/** The view moved by `bars` bars (negative: back). */
export function panBars(view: View, grid: Grid, sampleRate: number, bars: number): View {
  const bar = samplesPerBeat(grid, sampleRate) * Math.max(1, grid.meter.beatsPerBar);
  return { ...view, start: view.start + bars * bar };
}
