import {
  VOLUME_MAX_DB,
  VOLUME_MIN_DB,
  volumeAt,
  volumePos,
  volumeStep,
  volumeText,
} from './volume';

describe('the volume taper', () => {
  it('puts 0 dB at the right end and off at the left', () => {
    expect(volumeAt(1)).toBe(VOLUME_MAX_DB);
    expect(volumePos(0)).toBe(1);
    expect(volumeAt(0)).toBeNull();
    expect(volumePos(null)).toBe(0);
    expect(volumeAt(0.03)).toBeNull();
    expect(volumeAt(0.04)).toBe(VOLUME_MIN_DB);
    expect(volumeAt(1.4)).toBe(0);
    expect(volumeAt(-1)).toBeNull();
  });

  it('round-trips every tenth of a dB, and rises with the position', () => {
    for (let tenths = VOLUME_MIN_DB * 10; tenths <= 0; tenths++) {
      const db = tenths / 10;
      expect(volumeAt(volumePos(db))).toBeCloseTo(db, 9);
    }
    let last = -Infinity;
    for (let i = 4; i <= 100; i++) {
      const db = volumeAt(i / 100) ?? -Infinity;
      expect(db).toBeGreaterThanOrEqual(last);
      last = db;
    }
    // Most of the travel is where listening happens.
    expect(volumeAt(0.3)).toBe(-30);
  });

  it('steps 1 dB, from off up to -60 dB and below -60 dB to off', () => {
    expect(volumeStep(-12, 1)).toBe(-11);
    expect(volumeStep(-0.4, 1)).toBe(0);
    expect(volumeStep(0, 1)).toBe(0);
    expect(volumeStep(null, 1)).toBe(VOLUME_MIN_DB);
    expect(volumeStep(null, -1)).toBeNull();
    expect(volumeStep(-60, -1)).toBeNull();
    expect(volumeStep(-59.5, -1)).toBeNull();
    expect(volumeStep(-1, 1)).toBe(0);
    expect(Object.is(volumeStep(-1, 1), -0)).toBe(false);
  });

  it('reads -12.0 dB, 0.0 dB and Off', () => {
    expect(volumeText(-12)).toBe('−12.0 dB');
    expect(volumeText(0)).toBe('0.0 dB');
    expect(volumeText(null)).toBe('Off');
  });
});
