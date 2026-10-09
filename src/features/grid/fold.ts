/**
 * How the transport row folds as it narrows, by its own width: key hints first, then the
 * labels of controls that keep an icon, then the bar readout, while the volume slider moves
 * into a popover. Every width down to the smallest window (720 px) holds the whole row.
 */
export const TRANSPORT_FOLD = {
  hint: '@max-[1340px]:hidden',
  label: '@max-[1240px]:hidden',
  compact: '@max-[1040px]:hidden',
  tight: '@max-[900px]:hidden',
  /** Shown only where `tight` hides. */
  tightOnly: 'hidden @max-[900px]:block',
} as const;
