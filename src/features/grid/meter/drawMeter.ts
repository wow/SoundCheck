import { OVERVIEW_H, RULER_H } from '../draw';
import { SCALE_MIN, type Side, TICKS, levelY, zoneOf, zoneRuns } from './scale';

/**
 * The meter strip painter: a plain function of what to show, drawn on a canvas already scaled
 * by the device pixel ratio. The bar runs beside the waveform and residual lane, down to the
 * overview's top edge; the readouts sit above it (in a band a little taller than the ruler, for
 * two lines of 12 px numbers), the label below it in the overview band.
 */

/** Strip widths, CSS pixels: a normal window, and one narrower than 1000 px. */
export const STRIP_W = 40;
export const STRIP_W_NARROW = 16;
/** The readouts' band at the top: two 12 px lines. */
export const HEADER_H = Math.max(RULER_H, 32);
/** Inside the bar band: room for the +3 dB end and the -36 dB end. */
const PAD = 4;

export interface StripLayout {
  width: number;
  height: number;
  barTop: number;
  barBottom: number;
  /** The well the bar fills. */
  wellX: number;
  wellW: number;
  /** Scale numbers: their x and alignment, or null in a narrow strip. */
  numbers: { x: number; align: 'left' | 'right' } | null;
  /** Which way the scale (and the loudness arrow) sits from the well. */
  outer: -1 | 1;
}

export function stripLayout(side: Side, width: number, height: number): StripLayout {
  // The bar band is the waveform and the residual lane; never inverted in a tiny box.
  const barTop = HEADER_H + PAD;
  const barBottom = Math.max(barTop + 1, height - OVERVIEW_H - PAD);
  // IN is left of the waveform, so its scale is on the outer (left) side; OUT mirrors it.
  const outer = side === 'in' ? -1 : 1;
  if (width < STRIP_W) {
    const wellW = Math.max(2, width - 4);
    return { width, height, barTop, barBottom, wellX: 2, wellW, numbers: null, outer };
  }
  const wellW = 14;
  const wellX = side === 'in' ? width - 4 - wellW : 4;
  const numbers =
    side === 'in'
      ? { x: wellX - 6, align: 'right' as const }
      : { x: wellX + wellW + 6, align: 'left' as const };
  return { width, height, barTop, barBottom, wellX, wellW, numbers, outer };
}

export interface StripPalette {
  bg0: string;
  bg1: string;
  line: string;
  fg2: string;
  tp: string;
  lufs: string;
  ok: string;
  warn: string;
  err: string;
  mono: string;
}

export function readStripPalette(el: Element = document.documentElement): StripPalette {
  const css = getComputedStyle(el);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    bg0: v('--sc-bg-0', '#0c0e12'),
    bg1: v('--sc-bg-1', '#12151b'),
    line: v('--sc-line', '#2a303b'),
    fg2: v('--sc-fg-2', '#7d8594'),
    tp: v('--sc-meter-tp', '#fb7185'),
    lufs: v('--sc-meter-lufs', '#7dd3fc'),
    ok: v('--sc-ok', '#4ade80'),
    warn: v('--sc-warn', '#fbbf24'),
    err: v('--sc-err', '#f87171'),
    mono: v('--sc-font-mono', 'ui-monospace, monospace'),
  };
}

/** What the strip shows; every level in dB, null for none. */
export interface StripScene {
  bar: number | null;
  hold: number | null;
  momentary: number | null;
  /** OUT only: the loudness target (dashed) and the true-peak ceiling. */
  target: number | null;
  ceiling: number | null;
  /** The peak went over its limit: the hold tick turns red whatever its zone. */
  over: boolean;
}

export function drawStrip(
  ctx: CanvasRenderingContext2D,
  l: StripLayout,
  s: StripScene,
  p: StripPalette,
): void {
  const y = (db: number) => levelY(db, l.barTop, l.barBottom);
  const wellEnd = l.wellX + l.wellW;
  ctx.fillStyle = p.bg1;
  ctx.fillRect(0, 0, l.width, l.height);
  ctx.fillStyle = p.bg0;
  ctx.fillRect(l.wellX, l.barTop, l.wellW, l.barBottom - l.barTop);

  // The scale: a hairline across the well per tick, its number beside it.
  ctx.font = `9px ${p.mono}`;
  ctx.textBaseline = 'middle';
  for (const db of TICKS) {
    const ty = Math.round(y(db)) + 0.5;
    ctx.fillStyle = p.line;
    ctx.fillRect(l.wellX, ty - 0.5, l.wellW, 1);
    if (l.numbers) {
      ctx.fillStyle = p.fg2;
      ctx.textAlign = l.numbers.align;
      ctx.fillText(String(Math.abs(db)), l.numbers.x, ty);
    }
  }

  if (s.bar !== null && Number.isFinite(s.bar)) {
    const top = y(s.bar);
    for (const run of zoneRuns(s.bar)) {
      const from = y(run.from);
      const to = y(run.to);
      ctx.fillStyle = p[run.zone];
      ctx.fillRect(l.wellX, to, l.wellW, from - to);
    }
    // The ticks read through the bar as gaps.
    ctx.fillStyle = p.bg0;
    for (const db of TICKS) {
      const ty = Math.round(y(db));
      if (ty > top) ctx.fillRect(l.wellX, ty, l.wellW, 1);
    }
  }

  if (s.ceiling !== null) {
    ctx.globalAlpha = 0.75;
    ctx.fillStyle = p.tp;
    ctx.fillRect(0, Math.round(y(s.ceiling)), l.width, 1);
    ctx.globalAlpha = 1;
  }
  if (s.target !== null) {
    ctx.strokeStyle = p.lufs;
    ctx.globalAlpha = 0.8;
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 2]);
    ctx.beginPath();
    const ty = Math.round(y(s.target)) + 0.5;
    ctx.moveTo(0, ty);
    ctx.lineTo(l.width, ty);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.globalAlpha = 1;
  }

  if (s.hold !== null && Number.isFinite(s.hold) && s.hold >= SCALE_MIN) {
    ctx.fillStyle = s.over ? p.err : p[zoneOf(s.hold)];
    ctx.fillRect(l.wellX, Math.round(y(s.hold)) - 1, l.wellW, 2);
  }
  if (s.momentary !== null) {
    const my = Math.round(y(s.momentary));
    ctx.fillStyle = p.lufs;
    ctx.fillRect(l.wellX - 1, my - 1, l.wellW + 2, 2);
    // A small arrow on the scale side, pointing at the well.
    const edge = l.outer < 0 ? l.wellX - 1 : wellEnd + 1;
    const size = l.numbers ? 4 : 2;
    ctx.beginPath();
    ctx.moveTo(edge, my);
    ctx.lineTo(edge + l.outer * size, my - size);
    ctx.lineTo(edge + l.outer * size, my + size);
    ctx.closePath();
    ctx.fill();
  }
}
