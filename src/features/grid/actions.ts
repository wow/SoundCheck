import type { Grid, Meter } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import {
  NO_EDIT,
  beatOne,
  bpmInRange,
  chosenMeter,
  nudged,
  octaveStep,
  placedBarOne,
  tappedBpm,
  typedBpm,
} from './edit';
import { panBars, samplesPerBeat, snapped, zoomAround, zoomLimits } from './geometry';
import { nextReview } from './navigate';
import { useTrack } from './store';
import { tapBpm, tapped } from './tap';
import { useView } from './viewStore';

/**
 * What the grid view's keys and buttons do, over the track and view stores. Edits that need no
 * solving (a nudge, a placed bar 1, a typed tempo) pass an estimate so the lines move at once.
 */

function current(): { grid: Grid; rate: number } | null {
  const t = useTrack.getState();
  return t.grid && t.opened ? { grid: t.grid, rate: t.opened.sampleRate } : null;
}

export function octave(step: 1 | -1): void {
  const c = current();
  if (!c) return;
  const t = useTrack.getState();
  t.edit(octaveStep(t.edits.present, c.grid, step));
}

export function beatIsOne(beat: number): void {
  const c = current();
  if (!c) return;
  const k = beat - 1;
  if (k <= 0 || beat > c.grid.meter.beatsPerBar) return;
  const t = useTrack.getState();
  const anchor = Math.round(c.grid.anchor + k * samplesPerBeat(c.grid, c.rate));
  t.edit(beatOne(t.edits.present, c.grid, c.rate, k), { ...c.grid, anchor });
}

export function nudge(direction: 1 | -1, unit: 'ms' | '10ms' | 'beat'): void {
  const c = current();
  if (!c) return;
  const step =
    unit === 'beat' ? samplesPerBeat(c.grid, c.rate) : (c.rate / 1000) * (unit === '10ms' ? 10 : 1);
  const samples = Math.round(direction * step);
  const t = useTrack.getState();
  const anchor = Math.max(0, c.grid.anchor + samples);
  t.edit(nudged(t.edits.present, c.grid, samples), { ...c.grid, anchor });
}

/**
 * Bar 1 at the pointer (or the playhead while playing), on the nearest attack unless `free`.
 * Works without a grid too: with a typed BPM it is how a track with no beats found gets one.
 */
export function barOneHere(free: boolean): void {
  const t = useTrack.getState();
  if (!t.opened) return;
  const at = t.player.playing ? t.player.position : useView.getState().hover;
  if (at === null) return;
  const sample = free ? Math.round(at) : snapped(t.onsets, at, t.opened.sampleRate);
  t.edit(placedBarOne(t.edits.present, sample), t.grid ? { ...t.grid, anchor: sample } : undefined);
}

export function typeBpm(bpm: number): void {
  const t = useTrack.getState();
  if (!t.opened || !bpmInRange(bpm)) return;
  const estimate = t.grid ? { ...t.grid, bpm: Math.round(bpm * 100) / 100 } : undefined;
  t.edit(typedBpm(t.edits.present, t.grid, bpm), estimate);
}

let taps: number[] = [];

/**
 * One tap; from the fourth the tempo applies. The taps of one run make one undo step. Returns
 * the tapped tempo, if any yet.
 */
export function tap(now: number = performance.now()): number | null {
  taps = tapped(taps, now);
  const bpm = tapBpm(taps);
  const t = useTrack.getState();
  if (bpm !== null && bpmInRange(bpm) && t.opened) {
    const continuing = t.edits.present.tempoHint !== null && tapBpm(taps.slice(0, -1)) !== null;
    t.edit(tappedBpm(t.edits.present, t.grid, bpm), undefined, continuing);
  }
  return bpm;
}

export function chooseMeter(meter: Meter): void {
  const c = current();
  if (!c) return;
  const t = useTrack.getState();
  t.edit(chosenMeter(t.edits.present, c.grid, meter));
}

export function resetGrid(): void {
  useTrack.getState().edit(NO_EDIT);
}

/** Space: plays from bar 1 the first time, then pauses and resumes. */
export function playPause(): void {
  const t = useTrack.getState();
  if (t.player.playing) t.pause();
  else if (t.player.position === 0 && t.grid)
    t.play(
      Math.max(
        0,
        t.grid.anchor - Math.round(samplesPerBeat(t.grid, t.opened?.sampleRate ?? 44_100)),
      ),
    );
  else t.play();
}

/** Home: the playhead back to one beat before bar 1, where the first play starts, and bar 1 in view. */
export function toBarOne(): void {
  const c = current();
  if (!c) return;
  useTrack.getState().seek(Math.max(0, c.grid.anchor - Math.round(samplesPerBeat(c.grid, c.rate))));
  jumpTo(c.grid.anchor);
}

export function toggleClick(): void {
  const t = useTrack.getState();
  t.setClick(!t.click);
}

export function zoom(factor: number): void {
  const v = useView.getState();
  const t = useTrack.getState();
  const rate = t.opened?.sampleRate ?? 44_100;
  const [closest, widest] = zoomLimits(rate, t.opened?.frames ?? rate * 600, v.view.widthPx);
  v.setView(zoomAround(v.view, factor, v.view.widthPx / 2, closest, widest));
}

/** The whole track in view. */
export function fitWhole(): void {
  const t = useTrack.getState();
  if (!t.opened) return;
  const v = useView.getState();
  const [, widest] = zoomLimits(t.opened.sampleRate, t.opened.frames, v.view.widthPx);
  v.setView({ ...v.view, start: 0, samplesPerPx: widest });
}

export function pan(bars: number): void {
  const c = current();
  if (!c) return;
  const v = useView.getState();
  v.setView(panBars(v.view, c.grid, c.rate, bars));
}

/** Shows `sample` a fifth of the way into the view. */
export function jumpTo(sample: number): void {
  const v = useView.getState();
  v.setView({ ...v.view, start: sample - v.view.widthPx * 0.2 * v.view.samplesPerPx });
}

/** Opens the next row needing review; with none left, returns to the table. */
export async function nextInQueue(): Promise<void> {
  const t = useTrack.getState();
  // One open at a time: a track still opening (maybe being analysed again) finishes first.
  if (t.fileId === null || t.phase === 'opening') return;
  const lib = useLibrary.getState();
  const next = nextReview(lib.order, lib.rows, t.fileId);
  if (next === null) {
    await back('Every track that needed review has been looked at.');
    return;
  }
  lib.select(next);
  await t.open(next);
}

/** Enter: confirms the grid by ear, then opens the next row needing review. */
export async function confirmAndNext(): Promise<void> {
  if (await useTrack.getState().confirm()) await nextInQueue();
}

/** Esc: back to the table, the row selected; edits are saved. */
export async function back(notice: string | null = null): Promise<void> {
  const fileId = useTrack.getState().fileId;
  await useTrack.getState().close();
  const lib = useLibrary.getState();
  if (fileId !== null) lib.select(fileId);
  if (notice) lib.setNotice(notice);
}

/** Opens an analysed row in the grid view (Enter or a double-click in the table). */
export function openInGridView(fileId: number): void {
  const lib = useLibrary.getState();
  const row = lib.rows[fileId];
  if (!row?.analysis) return;
  lib.select(fileId);
  void useTrack.getState().open(fileId);
}
