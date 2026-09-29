import { describe, expect, it } from 'vitest';
import {
  NO_EDIT,
  beatOne,
  changes,
  chosenMeter,
  history,
  isEmpty,
  nudged,
  octaveStep,
  placedBarOne,
  push,
  redo,
  tappedBpm,
  typedBpm,
  undo,
  withFit,
} from './edit';
import { AKSAK, testGrid } from './testGrid';

const RATE = 48_000;
const grid = testGrid(120, 24_000);

describe('grid edits', () => {
  it('x2 and /2 step the octave, or double a typed or tapped tempo', () => {
    expect(octaveStep(NO_EDIT, grid, 1).octave).toBe(1);
    expect(octaveStep(octaveStep(NO_EDIT, grid, 1), grid, -1).octave).toBe(0);
    expect(octaveStep(typedBpm(NO_EDIT, grid, 64.5), grid, 1).bpm).toBe(129);
    expect(octaveStep(tappedBpm(NO_EDIT, grid, 61.7), grid, -1).tempoHint).toBeCloseTo(30.85);
  });

  it('a typed tempo is kept to two decimals and replaces a tap and octave steps', () => {
    const e = typedBpm(octaveStep(tappedBpm(NO_EDIT, grid, 120.4), grid, 1), grid, 127.984);
    expect(e).toMatchObject({ bpm: 127.98, tempoHint: null, octave: 0 });
  });

  it('beat-1 keys shift the solved bar, or move a placed bar 1 by whole beats', () => {
    const shifted = beatOne(beatOne(NO_EDIT, grid, RATE, 2), grid, RATE, 3);
    expect(shifted.downbeatShift).toBe(1);
    const aksak = testGrid(399, 0, AKSAK);
    expect(beatOne(NO_EDIT, aksak, RATE, 8).downbeatShift).toBe(8);
    const placed = placedBarOne(NO_EDIT, 30_000);
    expect(beatOne(placed, testGrid(120, 30_000), RATE, 3).anchor).toBe(30_000 + 3 * 24_000);
    expect(beatOne(NO_EDIT, grid, RATE, 0)).toBe(NO_EDIT);
  });

  it('a nudge pins bar 1, and tempo or meter changes pin a shifted bar 1 first', () => {
    expect(nudged(NO_EDIT, grid, 48)).toMatchObject({ anchor: 24_048, downbeatShift: 0 });
    expect(nudged(NO_EDIT, testGrid(120, 10), -480).anchor).toBe(0);
    const shifted = beatOne(NO_EDIT, grid, RATE, 1);
    // The grid shown has bar 1 on the shifted beat; x2 keeps it there.
    const shown = testGrid(120, 48_000);
    expect(octaveStep(shifted, shown, 1)).toMatchObject({
      octave: 1,
      anchor: 48_000,
      downbeatShift: 0,
    });
    expect(chosenMeter(shifted, shown, AKSAK)).toMatchObject({
      meter: AKSAK,
      anchor: 48_000,
      downbeatShift: 0,
    });
  });

  it('undo and redo walk the history; an unchanged edit adds no step', () => {
    let h = history(NO_EDIT);
    const steps = [
      octaveStep(NO_EDIT, grid, 1),
      nudged(NO_EDIT, grid, 48),
      typedBpm(NO_EDIT, grid, 128),
    ];
    for (const s of steps) h = push(h, s);
    expect(push(h, steps[2]!)).toBe(h);
    h = undo(undo(h));
    expect(h.present).toEqual(steps[0]);
    h = redo(h);
    expect(h.present).toEqual(steps[1]);
    h = push(h, NO_EDIT);
    expect(h.future).toEqual([]);
    expect(undo(history(NO_EDIT)).present).toBe(NO_EDIT);
    expect(isEmpty(h.present)).toBe(true);
  });

  it('describes the changes against the analysis', () => {
    const edited = testGrid(240, 24_144);
    const edit = nudged(octaveStep(NO_EDIT, grid, 1), grid, 144);
    expect(changes(grid, edited, edit, RATE)).toEqual(['×2 → 240.00', 'bar 1 +3 ms']);
    expect(changes(grid, grid, NO_EDIT, RATE)).toEqual([]);
    const meter = chosenMeter(NO_EDIT, grid, AKSAK);
    expect(changes(grid, testGrid(399, 24_000, AKSAK), meter, RATE)).toEqual([
      'meter 4/4 → 9/8 · 2+2+2+3',
    ]);
  });

  it('stops at the engine limits: 3 octave steps, 20-1000 BPM', () => {
    let e = NO_EDIT;
    for (let i = 0; i < 5; i++) e = octaveStep(e, grid, 1);
    expect(e.octave).toBe(3);
    expect(octaveStep(typedBpm(NO_EDIT, grid, 600), grid, 1).bpm).toBe(600);
    expect(octaveStep(typedBpm(NO_EDIT, grid, 30), grid, -1).bpm).toBe(30);
    expect(typedBpm(NO_EDIT, null, 128).bpm).toBe(128);
  });

  it('lists a beat-1 shift or a slower octave once, not also as a moved bar 1', () => {
    const shift = beatOne(NO_EDIT, grid, RATE, 1);
    const shifted = testGrid(120, grid.anchor + 24_000); // one beat at 120 BPM, 48 kHz
    expect(changes(grid, shifted, shift, RATE)).toEqual(['beat 1 = 2']);
    // A slower lattice re-anchors on the base tempo's bar 1, which the user did not move.
    const half = octaveStep(NO_EDIT, grid, -1);
    expect(changes(grid, testGrid(60, grid.anchor + 480), half, RATE)).toEqual(['½ → 60.00']);
  });
});

describe('the start fit', () => {
  it('is an override like the others, shown as a change', () => {
    const grid = testGrid(117.23, 24_000);
    const start = withFit(NO_EDIT, 'start');
    expect(start).toEqual({ ...NO_EDIT, fit: 'start' });
    expect(isEmpty(start)).toBe(false);
    expect(withFit(start, 'start')).toBe(start);
    expect(isEmpty(withFit(start, 'whole'))).toBe(true);
    expect(changes(grid, testGrid(117.02, 23_100), start, RATE)).toEqual(['fitted to the start']);
    // Other fixes keep it.
    expect(octaveStep(start, grid, -1).fit).toBe('start');
  });
});
