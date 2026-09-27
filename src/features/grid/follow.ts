import { type View, sampleToX } from './geometry';

/**
 * How the view follows the playhead while playing, and the playhead's clock between the
 * engine's position reports (30 a second). Pure, so the behaviour is tested without a canvas.
 */

/** Where the playhead stays while the waveform scrolls under it, as a share of the width. */
export const FOLLOW_AT = 0.5;
/** With reduced motion the view pages instead: on at this share, to `PAGE_TO`. */
export const PAGE_AT = 0.85;
export const PAGE_TO = 0.15;

/**
 * The view while playing: the playhead moves freely up to `FOLLOW_AT` of the width, then the
 * waveform scrolls under it. The view moves in whole device pixels, so each column shows the
 * same bins as its neighbour did a frame before and the waveform does not shimmer. A playhead
 * outside the view (after a seek or a pan) is brought straight back.
 */
export function followed(view: View, head: number, devicePixelRatio: number): View {
  const x = sampleToX(view, head);
  const anchor = Math.round(view.widthPx * FOLLOW_AT);
  if (x >= 0 && x <= anchor) return view;
  const devicePx = view.samplesPerPx / Math.max(1, devicePixelRatio);
  const start = Math.round((head - anchor * view.samplesPerPx) / devicePx) * devicePx;
  return start === view.start ? view : { ...view, start };
}

/** The view while playing with reduced motion: a page on when the playhead passes `PAGE_AT`. */
export function paged(view: View, head: number): View {
  const x = sampleToX(view, head);
  if (x >= 0 && x <= view.widthPx * PAGE_AT) return view;
  return { ...view, start: head - view.widthPx * PAGE_TO * view.samplesPerPx };
}

/** The playhead's clock: the frame heard at time `at` (ms, `performance.now`). */
export interface Heard {
  position: number;
  at: number;
  playing: boolean;
}

/** A report further than this from the clock is a seek, not jitter. */
const SEEK_S = 0.05;
/** The share of a small difference the clock takes up per report. */
const EASE = 0.1;

/** Where the clock says the playhead is at time `now`. */
export function heardAt(h: Heard, now: number, sampleRate: number): number {
  return h.playing ? h.position + ((now - h.at) / 1000) * sampleRate : h.position;
}

/**
 * The clock after a position report. The engine reports the frame being heard in steps of its
 * output buffer, so a report is a few milliseconds early or late; taken as it is, it would
 * shake the playhead and, while the view follows, the whole waveform. While playing, a small
 * difference is eased out over several reports; a large one (a seek) or a start or stop is
 * taken as it is.
 */
export function reported(
  h: Heard,
  report: { position: number; playing: boolean },
  now: number,
  sampleRate: number,
): Heard {
  if (!(h.playing && report.playing))
    return { position: report.position, at: now, playing: report.playing };
  const predicted = heardAt(h, now, sampleRate);
  const error = report.position - predicted;
  if (Math.abs(error) > SEEK_S * sampleRate)
    return { position: report.position, at: now, playing: true };
  return { position: predicted + error * EASE, at: now, playing: true };
}
