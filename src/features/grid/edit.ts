import type { EditFit, Grid, GridEdit, Meter } from '@/lib/ipc';
import { samplesPerBeat } from './geometry';
import { meterText } from './meters';

/**
 * How each fix changes the saved edit. The engine applies an edit as overrides on the analysis
 * (`sc_analysis::refit`); these rules compose a new key press with the edit so far, pure so they
 * are tested without the engine. A beat-1 choice is relative to the bar the engine solves, so
 * any change of tempo or meter first pins bar 1 where it is, keeping it where the user put it.
 */

export const NO_EDIT: GridEdit = {
  meter: null,
  bpm: null,
  tempoHint: null,
  octave: 0,
  anchor: null,
  downbeatShift: 0,
  fit: 'whole',
};

export function isEmpty(edit: GridEdit): boolean {
  return (
    edit.meter === null &&
    edit.bpm === null &&
    edit.tempoHint === null &&
    edit.octave === 0 &&
    edit.anchor === null &&
    edit.downbeatShift === 0 &&
    edit.fit === 'whole'
  );
}

/**
 * Fits the grid to the whole track or to its start (up to its first 128 beats, fewer when the
 * tempo changes sooner), for a track whose tempo changes: the other overrides still apply on top.
 */
export function withFit(edit: GridEdit, fit: EditFit): GridEdit {
  return edit.fit === fit ? edit : { ...edit, fit };
}

/** The tempi and octave steps the engine accepts in an edit. */
export const MIN_BPM = 20;
export const MAX_BPM = 1000;
export const MAX_OCTAVE_STEPS = 3;

export function bpmInRange(bpm: number): boolean {
  return bpm >= MIN_BPM && bpm <= MAX_BPM;
}

/** Bar 1 pinned where `grid` has it, so a tempo or meter change cannot move it. */
function pinned(edit: GridEdit, grid: Grid | null): GridEdit {
  return edit.downbeatShift === 0 || grid === null
    ? edit
    : { ...edit, anchor: grid.anchor, downbeatShift: 0 };
}

/**
 * x2 (+1) or /2 (-1): doubles a typed BPM, else steps the octave. Past the engine's limits
 * (20-1000 BPM, 3 steps either way) the edit stays as it is.
 */
export function octaveStep(edit: GridEdit, grid: Grid, step: 1 | -1): GridEdit {
  const e = pinned(edit, grid);
  const scaled = (bpm: number) => (step > 0 ? bpm * 2 : bpm / 2);
  if (e.bpm !== null) return bpmInRange(scaled(e.bpm)) ? { ...e, bpm: scaled(e.bpm) } : edit;
  if (e.tempoHint !== null)
    return bpmInRange(scaled(e.tempoHint)) ? { ...e, tempoHint: scaled(e.tempoHint) } : edit;
  return Math.abs(e.octave + step) <= MAX_OCTAVE_STEPS ? { ...e, octave: e.octave + step } : edit;
}

/** A typed tempo, kept exactly (`grid` is null for a track with no beats found). */
export function typedBpm(edit: GridEdit, grid: Grid | null, bpm: number): GridEdit {
  return { ...pinned(edit, grid), bpm: Math.round(bpm * 100) / 100, tempoHint: null, octave: 0 };
}

/** A tapped tempo: the engine takes the lattice of the fitted tempo nearest to it. */
export function tappedBpm(edit: GridEdit, grid: Grid | null, bpm: number): GridEdit {
  return { ...pinned(edit, grid), bpm: null, tempoHint: bpm, octave: 0 };
}

/** A meter from the picker. */
export function chosenMeter(edit: GridEdit, grid: Grid, meter: Meter): GridEdit {
  return { ...pinned(edit, grid), meter };
}

/** Beat `k` (0-based) of the current bar becomes beat 1. */
export function beatOne(edit: GridEdit, grid: Grid, sampleRate: number, k: number): GridEdit {
  if (k <= 0) return edit;
  if (edit.anchor !== null) {
    return { ...edit, anchor: Math.round(grid.anchor + k * samplesPerBeat(grid, sampleRate)) };
  }
  const bpb = Math.max(1, grid.meter.beatsPerBar);
  return { ...edit, downbeatShift: (edit.downbeatShift + k) % bpb };
}

/** Bar 1 on sample `sample` (the engine counts it back to the first bar). */
export function placedBarOne(edit: GridEdit, sample: number): GridEdit {
  return { ...edit, anchor: Math.max(0, Math.round(sample)), downbeatShift: 0 };
}

/** Bar 1 moved by `samples` (a nudge). */
export function nudged(edit: GridEdit, grid: Grid, samples: number): GridEdit {
  return placedBarOne(edit, grid.anchor + samples);
}

/** Undo history of edits: the present one and the steps either side. */
export interface History {
  past: GridEdit[];
  present: GridEdit;
  future: GridEdit[];
}

export function history(present: GridEdit): History {
  return { past: [], present, future: [] };
}

export function push(h: History, next: GridEdit): History {
  if (JSON.stringify(next) === JSON.stringify(h.present)) return h;
  return { past: [...h.past, h.present], present: next, future: [] };
}

export function undo(h: History): History {
  const prev = h.past[h.past.length - 1];
  return prev === undefined
    ? h
    : { past: h.past.slice(0, -1), present: prev, future: [h.present, ...h.future] };
}

export function redo(h: History): History {
  const [next, ...rest] = h.future;
  return next === undefined ? h : { past: [...h.past, h.present], present: next, future: rest };
}

/** The change chips: what the edited grid does differently from the analysis. */
export function changes(
  analysed: Grid | null,
  grid: Grid | null,
  edit: GridEdit,
  sampleRate: number,
): string[] {
  if (isEmpty(edit) || grid === null) return [];
  const chips: string[] = [];
  if (
    edit.meter !== null &&
    (analysed === null || meterText(analysed.meter) !== meterText(edit.meter))
  ) {
    chips.push(`meter ${analysed ? meterText(analysed.meter) : '–'} → ${meterText(edit.meter)}`);
  }
  if (edit.bpm !== null) chips.push(`BPM ${grid.bpm.toFixed(2)} (typed)`);
  else if (edit.tempoHint !== null)
    chips.push(`tap ${edit.tempoHint.toFixed(1)} → ${grid.bpm.toFixed(2)}`);
  else if (edit.octave !== 0)
    chips.push(`${edit.octave > 0 ? '×2' : '½'} → ${grid.bpm.toFixed(2)}`);
  // Only a bar 1 the user placed or nudged: a beat-1 shift or a slower octave also moves the
  // anchor, and has its own chip.
  if (edit.anchor !== null && analysed !== null && grid.anchor !== analysed.anchor) {
    const ms = ((grid.anchor - analysed.anchor) / sampleRate) * 1000;
    const within = Math.abs(ms) < 1000;
    chips.push(within ? `bar 1 ${ms > 0 ? '+' : ''}${ms.toFixed(0)} ms` : 'bar 1 moved');
  }
  if (edit.downbeatShift !== 0) chips.push(`beat 1 = ${edit.downbeatShift + 1}`);
  if (edit.fit === 'start') chips.push('fitted to the start');
  return chips;
}
