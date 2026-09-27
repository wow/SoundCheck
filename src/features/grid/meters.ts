import type { Meter } from '@/lib/ipc';

/** The meters the picker offers, as the engine's estimator knows them. */
export const METERS: Meter[] = [
  { beatsPerBar: 4, unit: 'quarter', grouping: [1, 1, 1, 1] },
  { beatsPerBar: 3, unit: 'quarter', grouping: [1, 1, 1] },
  { beatsPerBar: 6, unit: 'eighth', grouping: [3, 3] },
  { beatsPerBar: 9, unit: 'eighth', grouping: [2, 2, 2, 3] },
  { beatsPerBar: 9, unit: 'eighth', grouping: [3, 2, 2, 2] },
  { beatsPerBar: 5, unit: 'eighth', grouping: [2, 3] },
  { beatsPerBar: 7, unit: 'eighth', grouping: [2, 2, 3] },
  { beatsPerBar: 10, unit: 'eighth', grouping: [3, 2, 2, 3] },
];

/** `4/4`, `6/8 · 3+3`, `9/8 · 2+2+2+3`: the badge text, as the engine prints it. */
export function meterText(m: Meter): string {
  const base =
    m.unit === 'quarter'
      ? `${m.beatsPerBar}/4`
      : m.unit === 'eighth'
        ? `${m.beatsPerBar}/8`
        : `${m.beatsPerBar * 3}/8`;
  return m.grouping.length > 1 && m.grouping.some((g) => g > 1)
    ? `${base} · ${m.grouping.join('+')}`
    : base;
}

export function sameMeter(a: Meter, b: Meter): boolean {
  return a.unit === b.unit && a.grouping.join('+') === b.grouping.join('+');
}

/** The picker's list: the runner-up first, then the rest, the current meter left out. */
export function pickerMeters(current: Meter, runnerUp: Meter | null): Meter[] {
  const rest = METERS.filter(
    (m) => !sameMeter(m, current) && !(runnerUp && sameMeter(m, runnerUp)),
  );
  return runnerUp && !sameMeter(runnerUp, current) ? [runnerUp, ...rest] : rest;
}
