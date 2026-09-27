/** How the table lays out its columns at the width it has. */
export interface Layout {
  compact: boolean;
  /**
   * Two-line cells for a narrow table: true peak under the loudness (no TP column), the meter
   * and the tempo flags under the BPM.
   */
  stacked: boolean;
  /**
   * Fixed column widths in px; `spec: null` hides the Spec column, `tp: null` the TP column.
   * Name takes the rest.
   */
  spec: number | null;
  loudness: number;
  tp: number | null;
  bpm: number;
  action: number;
  status: number;
  /** The Name column never gets narrower than this; below it the table scrolls sideways. */
  nameMin: number;
  /** Whether the loudness cell draws its small delta bar. */
  bar: boolean;
}

/** The full layout, from the approved design. */
export const WIDE: Layout = {
  compact: false,
  stacked: false,
  spec: 112,
  loudness: 188,
  tp: 56,
  bpm: 196,
  action: 224,
  status: 118,
  nameMin: 180,
  bar: true,
};

/**
 * For smaller windows: no Spec column (the not-DJ-safe `!` moves next to the format badge), no
 * loudness bar, narrower BPM (its meter badge shows only the time signature), Action and Status.
 */
export const COMPACT: Layout = {
  compact: true,
  stacked: false,
  spec: null,
  loudness: 124,
  tp: 56,
  bpm: 176,
  action: 200,
  status: 112,
  nameMin: 150,
  bar: false,
};

/**
 * For narrow windows (down to 720 px, the table then as wide as the window): two-line cells,
 * so every row still shows loudness, true peak, BPM, meter, Action and Status without
 * scrolling sideways.
 */
export const NARROW: Layout = {
  compact: true,
  stacked: true,
  spec: null,
  loudness: 124,
  tp: null,
  bpm: 132,
  action: 180,
  status: 112,
  nameMin: 140,
  bar: false,
};

/** Table widths below which the compact and the narrow layouts are used. */
export const COMPACT_BELOW = minWidth(WIDE) + 20;
export const NARROW_BELOW = minWidth(COMPACT) + 20;

/** The layout for a table `width` px wide; unknown widths (0) get the wide one. */
export function layoutFor(width: number): Layout {
  if (width <= 0 || width >= COMPACT_BELOW) return WIDE;
  return width < NARROW_BELOW ? NARROW : COMPACT;
}

/** The narrowest the table can be before it scrolls sideways. */
export function minWidth(l: Layout): number {
  return l.nameMin + (l.spec ?? 0) + l.loudness + (l.tp ?? 0) + l.bpm + l.action + l.status;
}

/** The DOM id of a row, for `aria-activedescendant`. */
export const rowDomId = (id: number) => `track-row-${id}`;
