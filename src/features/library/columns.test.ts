import { COMPACT, COMPACT_BELOW, NARROW, NARROW_BELOW, WIDE, layoutFor, minWidth } from './columns';

describe('table layout', () => {
  it('goes wide, compact, then narrow as the table narrows', () => {
    expect(layoutFor(0)).toBe(WIDE);
    expect(layoutFor(1200)).toBe(WIDE);
    expect(layoutFor(COMPACT_BELOW - 1)).toBe(COMPACT);
    expect(layoutFor(NARROW_BELOW)).toBe(COMPACT);
    expect(layoutFor(NARROW_BELOW - 1)).toBe(NARROW);
    expect(layoutFor(700)).toBe(NARROW);
  });

  it('each layout fits where it is used', () => {
    expect(minWidth(WIDE)).toBeLessThan(COMPACT_BELOW);
    expect(minWidth(COMPACT)).toBeLessThan(NARROW_BELOW);
    // The narrow table fills a 720 px window: less a scrollbar, and some slack.
    expect(minWidth(NARROW)).toBeLessThanOrEqual(704);
    expect(COMPACT.spec).toBeNull();
    expect(NARROW.tp).toBeNull();
    expect(NARROW.stacked).toBe(true);
  });
});
