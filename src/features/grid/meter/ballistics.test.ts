import type { MeterFrame } from '@/lib/ipc';
import {
  FALL_DB_PER_MS,
  HOLD_MS,
  NO_OVERS,
  NO_PEAK,
  barLevel,
  holdLevel,
  latched,
  peakInput,
  smoothed,
} from './ballistics';
import { SCALE_MAX, SCALE_MIN, caption, captionText, levelY, readingText } from './scale';
import { analysis } from '@/test/fixtures';

function frame(inPeak: number | null, outPeak: number | null): MeterFrame {
  return { position: 0, inPeak, inMomentary: null, outPeak, outMomentary: null, folded: false };
}

describe('peak ballistics', () => {
  it('falls 20 dB in 1.7 s', () => {
    expect(FALL_DB_PER_MS * 1700).toBeCloseTo(20, 12);
  });

  it('takes a peak at once, then the bar falls 20 dB per 1.7 s', () => {
    const p = peakInput(NO_PEAK, -6, 1000);
    expect(barLevel(p, 1000)).toBe(-6);
    expect(barLevel(p, 1000 + 1700)).toBeCloseTo(-26, 9);
    expect(barLevel(p, 1000 + 850)).toBeCloseTo(-16, 9);
    // A lower reading under the falling bar leaves it falling; a higher one takes it again.
    expect(peakInput(p, -30, 1100)).toBe(p);
    expect(barLevel(peakInput(p, -3, 1100), 1100)).toBe(-3);
  });

  it('holds the peak tick for 1.5 s, then lets it fall at the same rate', () => {
    let p = peakInput(NO_PEAK, -1, 0);
    // Lower peaks during the hold do not move it.
    p = peakInput(p, -10, 500);
    p = peakInput(p, -4, 1400);
    expect(holdLevel(p, HOLD_MS)).toBe(-1);
    expect(holdLevel(p, HOLD_MS + 1700)).toBeCloseTo(-21, 9);
    // The bar meanwhile followed the -4 peak and fell from there.
    expect(barLevel(p, 1400)).toBe(-4);
    // A peak above the falling tick holds again from its own time.
    const later = peakInput(p, -8, HOLD_MS + 1700);
    expect(holdLevel(later, HOLD_MS + 1700 + HOLD_MS)).toBe(-8);
  });

  it('ignores digital silence: no reading is no peak', () => {
    const p = peakInput(NO_PEAK, -12, 0);
    expect(peakInput(p, null, 10)).toBe(p);
    expect(barLevel(NO_PEAK, 0)).toBe(-Infinity);
  });
});

describe('loudness smoothing', () => {
  it('moves 63 % of the way in 100 ms, shows a first reading at once, hides on none', () => {
    expect(smoothed(null, -12, 16)).toBe(-12);
    expect(smoothed(-20, -10, 100)).toBeCloseTo(-20 + 10 * (1 - Math.exp(-1)), 9);
    expect(smoothed(-20, -10, 0)).toBe(-20);
    expect(smoothed(-20, null, 16)).toBeNull();
  });
});

describe('the over latch', () => {
  it('latches IN over 0 dBTP and OUT over the ceiling until cleared', () => {
    const ceiling = -0.5;
    expect(latched(NO_OVERS, frame(-0.1, -2.6), ceiling)).toBe(NO_OVERS);
    // At the ceiling is not over it.
    expect(latched(NO_OVERS, frame(0, -0.5), ceiling)).toBe(NO_OVERS);
    const out = latched(NO_OVERS, frame(-0.1, -0.4), ceiling);
    expect(out).toEqual({ in: false, out: true });
    const both = latched(out, frame(0.3, -2), ceiling);
    expect(both).toEqual({ in: true, out: true });
    // Back under: still latched; no frame changes nothing.
    expect(latched(both, frame(-20, -22), ceiling)).toBe(both);
    expect(latched(both, null, ceiling)).toBe(both);
    expect(latched(NO_OVERS, frame(null, null), ceiling)).toBe(NO_OVERS);
  });
});

describe('the meter scale and captions', () => {
  it('maps +3 dB to the top and -36 dB to the bottom, clamped', () => {
    expect(levelY(SCALE_MAX, 10, 400)).toBe(10);
    expect(levelY(SCALE_MIN, 10, 400)).toBe(400);
    expect(levelY(-60, 10, 400)).toBe(400);
    expect(levelY(-16.5, 10, 400)).toBeCloseTo(205, 9);
  });

  it('captions IN with the analysis and OUT with the planned values, per mode', () => {
    const a = analysis({ shortTermP95: -9.2, integrated: -10.4, truePeak: -0.3 });
    expect(captionText(caption('in', a, 'dj', -1.8))).toBe('S-P95 -9.2 · TP -0.3');
    expect(captionText(caption('out', a, 'dj', -1.8))).toBe('S-P95 -11.0 · TP -2.1');
    expect(captionText(caption('out', a, 'streaming', -3.6))).toBe('Integrated -14.0 · TP -3.9');
    expect(caption('out', analysis({ shortTermP95: null }), 'dj', 0).loudness).toBeNull();
    expect(caption('in', undefined, 'dj', 0)).toEqual({
      stat: 'S-P95',
      loudness: null,
      truePeak: null,
    });
  });

  it('writes readings with a + above full scale and a dash for none', () => {
    expect(readingText(0.43, true)).toBe('+0.4');
    expect(readingText(-9.249, false)).toBe('-9.2');
    expect(readingText(null, true)).toBe('–');
    expect(readingText(-Infinity, true)).toBe('–');
  });
});
