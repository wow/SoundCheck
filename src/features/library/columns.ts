/** How the table lays out its columns at the width it has. */
export interface Layout {
  compact: boolean;
  /** Fixed column widths in px; `spec: null` hides the Spec column. Name takes the rest. */
  spec: number | null;
  loudness: number;
  tp: number;
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
  spec: 108,
  loudness: 168,
  tp: 56,
  bpm: 196,
  action: 224,
  status: 118,
  nameMin: 180,
  bar: true,
};

/**
 * For small windows: no Spec column (the not-DJ-safe `!` moves next to the format badge), no
 * loudness bar, narrower BPM, Action and Status.
 */
export const COMPACT: Layout = {
  compact: true,
  spec: null,
  loudness: 118,
  tp: 56,
  bpm: 176,
  action: 200,
  status: 112,
  nameMin: 150,
  bar: false,
};

/** Table width below which the compact layout is used; the wide one needs this much. */
export const COMPACT_BELOW = minWidth(WIDE) + 20;

/** The layout for a table `width` px wide; unknown widths (0) get the wide one. */
export function layoutFor(width: number): Layout {
  return width > 0 && width < COMPACT_BELOW ? COMPACT : WIDE;
}

/** The narrowest the table can be before it scrolls sideways. */
export function minWidth(l: Layout): number {
  return l.nameMin + (l.spec ?? 0) + l.loudness + l.tp + l.bpm + l.action + l.status;
}

/** The DOM id of a row, for `aria-activedescendant`. */
export const rowDomId = (id: number) => `track-row-${id}`;
