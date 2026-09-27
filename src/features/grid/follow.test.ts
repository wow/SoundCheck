import { type Heard, FOLLOW_AT, followed, heardAt, paged, reported } from './follow';
import { sampleToX } from './geometry';

const RATE = 48_000;
// 1,000 CSS px showing 100 samples each: 100,000 samples from 0.
const view = { start: 0, samplesPerPx: 100, widthPx: 1000 };

describe('following the playhead', () => {
  it('lets the playhead move to the middle, then scrolls under it', () => {
    expect(followed(view, 30_000, 2)).toBe(view);
    expect(followed(view, 50_000, 2)).toBe(view);
    const next = followed(view, 51_234, 2);
    // The playhead stays at the middle, within half a device pixel.
    expect(Math.abs(sampleToX(next, 51_234) - 1000 * FOLLOW_AT)).toBeLessThanOrEqual(0.25);
    // The view moves in whole device pixels (50 samples at 2x).
    expect(next.start % 50).toBe(0);
    expect(next.samplesPerPx).toBe(100);
  });

  it('brings back a playhead outside the view', () => {
    expect(sampleToX(followed({ ...view, start: 80_000 }, 10_000, 1), 10_000)).toBe(500);
    expect(sampleToX(followed(view, 400_000, 1), 400_000)).toBe(500);
  });

  it('pages instead with reduced motion', () => {
    expect(paged(view, 84_000)).toBe(view);
    expect(sampleToX(paged(view, 86_000), 86_000)).toBeCloseTo(150, 6);
  });
});

describe("the playhead's clock", () => {
  const playing: Heard = { position: 48_000, at: 1000, playing: true };

  it('runs on in real time between reports', () => {
    expect(heardAt(playing, 1500, RATE)).toBe(72_000);
    expect(heardAt({ ...playing, playing: false }, 1500, RATE)).toBe(48_000);
  });

  it('eases out a report a few milliseconds off, instead of jumping to it', () => {
    // 500 ms on, the clock says 72,000; the report says 5 ms (240 frames) later.
    const h = reported(playing, { position: 72_240, playing: true }, 1500, RATE);
    expect(h.position).toBeCloseTo(72_024, 6);
    expect(h.at).toBe(1500);
  });

  it('takes a seek, a start or a stop as reported', () => {
    expect(reported(playing, { position: 960_000, playing: true }, 1500, RATE).position).toBe(
      960_000,
    );
    const paused = { ...playing, playing: false };
    expect(reported(paused, { position: 50_000, playing: true }, 1500, RATE)).toEqual({
      position: 50_000,
      at: 1500,
      playing: true,
    });
    expect(reported(playing, { position: 70_000, playing: false }, 1500, RATE)).toEqual({
      position: 70_000,
      at: 1500,
      playing: false,
    });
  });
});
