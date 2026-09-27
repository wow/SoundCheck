import { describe, expect, it } from 'vitest';
import {
  barResiduals,
  clockText,
  fitTones,
  groupOf,
  lineKind,
  linesInRange,
  nearestLine,
  residualTone,
  rulerLabel,
  sampleToX,
  samplesPerBin,
  xToSample,
  zoomAround,
} from './geometry';
import { AKSAK, FOUR_FOUR, testGrid } from './testGrid';

describe('grid view geometry', () => {
  it('maps samples to pixels and back, and zooms around the cursor', () => {
    const view = { start: 1000, samplesPerPx: 50, widthPx: 800 };
    expect(sampleToX(view, 6000)).toBe(100);
    expect(xToSample(view, 100)).toBe(6000);
    const zoomed = zoomAround(view, 2, 100, 1, 1e6);
    expect(zoomed.samplesPerPx).toBe(25);
    expect(xToSample(zoomed, 100)).toBe(6000);
    expect(zoomAround(view, 1000, 100, 8, 1e6).samplesPerPx).toBe(8);
  });

  it('lists lines with their accents and bar numbers, pickup bars negative', () => {
    // 120 BPM at 48 kHz: a beat every 24,000 samples, bar 1 at 60,000.
    const lines = linesInRange(testGrid(120, 60_000), 48_000, 0, 160_000);
    expect(lines.map((l) => l.sample)).toEqual([
      12_000, 36_000, 60_000, 84_000, 108_000, 132_000, 156_000,
    ]);
    expect(lines.map((l) => l.bar)).toEqual([-1, -1, 1, 1, 1, 1, 2]);
    expect(lines.map((l) => l.pulse)).toEqual([3, 4, 1, 2, 3, 4, 1]);
    expect(lines.map((l) => l.kind)).toEqual([
      'group',
      'group',
      'bar',
      'group',
      'group',
      'group',
      'bar',
    ]);
  });

  it('marks aksak group starts and the pulses between them', () => {
    const kinds = Array.from({ length: 9 }, (_, p) => lineKind(AKSAK, p));
    expect(kinds).toEqual([
      'bar',
      'pulse',
      'group',
      'pulse',
      'group',
      'pulse',
      'group',
      'pulse',
      'pulse',
    ]);
    expect(lineKind(FOUR_FOUR, 2)).toBe('group');
  });

  it('keeps a line that rounds onto the first sample of the range', () => {
    const grid = testGrid((60 * 48_000) / 20_479.7, 0);
    expect(linesInRange(grid, 48_000, 20_480, 30_000).map((l) => l.sample)).toEqual([20_480]);
    expect(nearestLine(grid, 48_000, 20_000)).toMatchObject({
      index: 1,
      sample: 20_480,
      bar: 1,
      pulse: 2,
    });
  });

  it('labels pulses by their place in the group', () => {
    const labels = (m: typeof AKSAK) =>
      Array.from({ length: m.beatsPerBar }, (_, k) => rulerLabel(m, k + 1));
    expect(labels(AKSAK).join(' ')).toBe('1 2 3 2 5 2 7 2 3');
    expect(labels(FOUR_FOUR).join(' ')).toBe('1 2 3 4');
    expect(labels({ beatsPerBar: 6, unit: 'eighth', grouping: [3, 3] }).join(' ')).toBe(
      '1 2 3 4 2 3',
    );
    expect([1, 2, 3, 5, 6, 7, 9].map((p) => groupOf(AKSAK, p))).toEqual([1, 1, 2, 3, 3, 4, 4]);
  });

  it('colours the fit against the verdict limits', () => {
    expect(fitTones(11.9, 29.9)).toEqual({ p95: 'ok', max: 'ok' });
    expect(fitTones(12, 30)).toEqual({ p95: 'warn', max: 'warn' });
    expect(fitTones(25, 50)).toEqual({ p95: 'err', max: 'err' });
  });

  it('colours residuals and keeps the worst per bar', () => {
    expect([3, -9.9, 10, -24, 25].map(residualTone)).toEqual(['ok', 'ok', 'warn', 'warn', 'err']);
    const r = new Float32Array([NaN, 2, -12, 4, NaN, NaN, NaN, NaN, NaN, 1]);
    // First line -1: lines -1 (bar -1), 0..3 (bar 1), 4..7 (bar 2), 8 (bar 3).
    const bars = barResiduals(r, -1, 4);
    expect(bars.get(-1)).toBeNaN();
    expect(bars.get(1)).toBe(-12);
    expect(bars.get(2)).toBeNaN();
    expect(bars.get(3)).toBe(1);
  });

  it('asks for the coarsest waveform level that still fills every device pixel', () => {
    expect(samplesPerBin(517, 2)).toBe(256);
    expect(samplesPerBin(5, 2)).toBe(8);
    expect(samplesPerBin(10_000, 1)).toBe(8192);
  });

  it('prints clock positions', () => {
    expect(clockText(0, 44_100)).toBe('0:00.000');
    expect(clockText(44_100 * 94.5, 44_100)).toBe('1:34.500');
  });
});
