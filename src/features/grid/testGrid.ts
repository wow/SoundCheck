import type { Grid, Meter } from '@/lib/ipc';

export const FOUR_FOUR: Meter = { beatsPerBar: 4, unit: 'quarter', grouping: [1, 1, 1, 1] };
export const AKSAK: Meter = { beatsPerBar: 9, unit: 'eighth', grouping: [2, 2, 2, 3] };

/** A grid for tests: `bpm` at the meter's unit, bar 1 at `anchor`. */
export function testGrid(bpm: number, anchor: number, meter: Meter = FOUR_FOUR): Grid {
  return {
    anchor,
    bpm,
    meter,
    meterRunnerUp: null,
    firstDownbeatIndex: 0,
    phraseLenBars: 8,
    segments: [],
    residualP95Ms: 3,
    residualMaxMs: 9,
    localBpmRange: 0.01,
    driftPpm: 4,
    verdict: 'static',
    confidence: 'green',
    reasons: [],
    alternatives: { octaveUp: bpm * 2, octaveDown: bpm / 2, downbeatShiftBeats: [] },
  };
}
