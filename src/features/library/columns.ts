/** Column widths in px; Name takes the rest. */
export const COLUMNS = {
  spec: 108,
  loudness: 168,
  tp: 56,
  bpm: 196,
  action: 224,
  status: 118,
} as const;

/** The Name column never gets narrower than this; below it the table scrolls sideways. */
export const NAME_MIN = 180;

/** The DOM id of a row, for `aria-activedescendant`. */
export const rowDomId = (id: number) => `track-row-${id}`;
