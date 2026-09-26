import { COMPACT, COMPACT_BELOW, WIDE, layoutFor, minWidth } from './columns';

describe('table layout', () => {
  it('switches to the compact layout when the wide one no longer fits', () => {
    expect(layoutFor(0)).toBe(WIDE);
    expect(layoutFor(1200)).toBe(WIDE);
    expect(layoutFor(COMPACT_BELOW - 1)).toBe(COMPACT);
    expect(layoutFor(820)).toBe(COMPACT);
  });

  it('the compact layout fits the table beside the rail in the smallest window', () => {
    // Window at its 1100 px minimum, the rail at 272 px.
    expect(minWidth(COMPACT)).toBeLessThanOrEqual(1100 - 272);
    expect(COMPACT.spec).toBeNull();
  });
});
