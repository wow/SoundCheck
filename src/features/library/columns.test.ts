import { COMPACT, COMPACT_BELOW, NARROW, NARROW_BELOW, WIDE, layoutFor, minWidth } from './columns';

/** The rail beside the table, in windows 1000 px wide or more. */
const RAIL = 272;

describe('table layout', () => {
  it('goes wide, compact, then narrow as the table narrows', () => {
    expect(layoutFor(0)).toBe(WIDE);
    expect(layoutFor(1440 - 300)).toBe(WIDE);
    expect(layoutFor(COMPACT_BELOW - 1)).toBe(COMPACT);
    expect(layoutFor(NARROW_BELOW)).toBe(COMPACT);
    expect(layoutFor(NARROW_BELOW - 1)).toBe(NARROW);
  });

  it('picks the layout that fits each window', () => {
    // 1100 px with the rail: compact, as before narrow windows were possible.
    expect(layoutFor(1100 - RAIL)).toBe(COMPACT);
    expect(minWidth(COMPACT)).toBeLessThanOrEqual(1100 - RAIL);
    // 1000 px, the narrowest with the rail: two-line cells.
    expect(layoutFor(1000 - RAIL)).toBe(NARROW);
    // 720 px, no rail: the narrow table fits, less a scrollbar and some slack.
    expect(layoutFor(720)).toBe(NARROW);
    expect(minWidth(NARROW)).toBeLessThanOrEqual(704);
    expect(NARROW.tp).toBeNull();
    expect(COMPACT.spec).toBeNull();
  });
});
