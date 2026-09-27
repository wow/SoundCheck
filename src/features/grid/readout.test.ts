import type { GridFit, TrackOpened } from '@/lib/ipc';
import { playText, readoutText } from './readout';
import { AKSAK, testGrid } from './testGrid';

const RATE = 48_000;
const grid = testGrid(240, 24_000, AKSAK); // a pulse every 12,000 samples
const opened = {
  sampleRate: RATE,
  timeline: { hopMs: 500, shortTerm: [-9.25, -8.5, -8.0, -7.5, -7.0, -6.5] },
} as unknown as TrackOpened;
const fit: GridFit = {
  header: { grid, firstLine: 0, lines: 6, worstLine: null, matched: 6, attacks: 6 },
  residuals: new Float32Array([1, -0.3, 2, 3, 4.6, Number.NaN]),
};
const idle = { playing: false, position: 0, underruns: 0, error: null };

describe('readouts', () => {
  it('reads time, pulse and group, residual and loudness at the pointer', () => {
    // Pulse 5 of 9/8 2+2+2+3 is line 4, at 72,000 samples (1.5 s), in group 3.
    expect(readoutText({ opened, grid, fit, player: idle }, 72_000)).toBe(
      '0:01.500 · bar 1 pulse 5 (group 3) · residual +5 ms · −7.5 LUFS-S',
    );
    // A residual rounding to zero never reads "-0"; an unmatched line has none.
    expect(readoutText({ opened, grid, fit, player: idle }, 36_000)).toContain('residual 0 ms');
    expect(readoutText({ opened, grid, fit, player: idle }, 84_000)).not.toContain('residual');
  });

  it('follows the playhead when the pointer is away, from the start of a track just opened', () => {
    const player = { ...idle, position: 48_000 };
    expect(readoutText({ opened, grid, fit, player }, null)).toMatch(/^0:01\.000 · bar 1 pulse 3/);
    // Bar 1 is at 0.5 s: the start is pulse 8 of the pickup bar before it.
    expect(readoutText({ opened, grid, fit, player: idle }, null)).toMatch(
      /^0:00\.000 · bar -1 pulse 8/,
    );
    expect(readoutText({ opened: null, grid, fit, player }, null)).toBeNull();
    expect(playText({ opened, grid, player })).toEqual({ clock: '0:01.000', bar: '1.3' });
    expect(playText({ opened, grid, player: idle })).toEqual({ clock: '0:00.000', bar: '-1.8' });
    expect(playText({ opened, grid: null, player: idle }).bar).toBe('–');
  });
});
