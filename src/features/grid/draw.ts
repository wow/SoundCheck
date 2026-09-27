import type { Grid, GridFit } from '@/lib/ipc';
import {
  type View,
  barResiduals,
  linesInRange,
  pxPerBeat,
  residualTone,
  rulerLabel,
  sampleToX,
  samplesPerBin,
} from './geometry';
import type { PeakTiles } from './peaks';

/**
 * The grid view's painters: plain functions of a scene, drawn on a canvas already scaled by the
 * device pixel ratio. Bands from the top: bar ruler, waveform with the grid, residual lane,
 * overview of the whole track.
 */

export const RULER_H = 26;
export const LANE_H = 52;
export const OVERVIEW_H = 28;
/** A residual this large fills half the lane: the attack match window. */
const LANE_SCALE_MS = 50;
/** Pixels per beat from which the ruler numbers every beat or pulse. */
const BEAT_LABEL_PX = 22;

export interface Bands {
  width: number;
  height: number;
  waveTop: number;
  waveBottom: number;
  laneTop: number;
  overviewTop: number;
}

export function bands(width: number, height: number): Bands {
  const overviewTop = height - OVERVIEW_H;
  const laneTop = overviewTop - LANE_H;
  return { width, height, waveTop: RULER_H, waveBottom: laneTop, laneTop, overviewTop };
}

export interface Palette {
  bg0: string;
  bg1: string;
  line: string;
  fg1: string;
  fg2: string;
  accent: string;
  teal: string;
  ok: string;
  warn: string;
  err: string;
  waveOrig: string;
  waveProc: string;
}

/** The palette from the design tokens. */
export function readPalette(el: Element = document.documentElement): Palette {
  const css = getComputedStyle(el);
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  return {
    bg0: v('--sc-bg-0', '#0c0e12'),
    bg1: v('--sc-bg-1', '#12151b'),
    line: v('--sc-line', '#2a303b'),
    fg1: v('--sc-fg-1', '#b8bec9'),
    fg2: v('--sc-fg-2', '#7d8594'),
    accent: v('--sc-accent', '#f5b942'),
    teal: v('--sc-accent-2', '#4fd1c5'),
    ok: v('--sc-ok', '#4ade80'),
    warn: v('--sc-warn', '#fbbf24'),
    err: v('--sc-err', '#f87171'),
    waveOrig: v('--sc-wave-orig', '#5b6472'),
    waveProc: v('--sc-wave-proc', '#c9d1dd'),
  };
}

export interface Scene {
  view: View;
  sampleRate: number;
  frames: number;
  grid: Grid | null;
  /** The analysed grid, drawn dashed under the edited one. */
  ghost: Grid | null;
  fit: GridFit | null;
  /** The planned gain as a linear factor, for the lighter inner waveform. */
  gain: number;
  peaks: PeakTiles;
  dpr: number;
}

const TEAL = (a: number) => `rgba(79, 209, 197, ${a})`;

export function drawScene(ctx: CanvasRenderingContext2D, b: Bands, s: Scene, p: Palette): void {
  ctx.fillStyle = p.bg0;
  ctx.fillRect(0, 0, b.width, b.height);
  ctx.fillStyle = p.bg1;
  ctx.fillRect(0, 0, b.width, RULER_H);
  ctx.fillRect(0, b.laneTop, b.width, b.height - b.laneTop);
  drawWave(ctx, b, s, p);
  if (s.ghost && s.grid && (s.ghost.anchor !== s.grid.anchor || s.ghost.bpm !== s.grid.bpm)) {
    drawGhost(ctx, b, s, s.ghost, p);
  }
  if (s.grid) {
    drawGrid(ctx, b, s, s.grid, p);
    drawRuler(ctx, s, s.grid, p);
    drawLane(ctx, b, s, s.grid, p);
  }
  drawOverview(ctx, b, s, p);
  ctx.strokeStyle = p.line;
  ctx.lineWidth = 1;
  for (const y of [RULER_H, b.laneTop, b.overviewTop]) {
    ctx.beginPath();
    ctx.moveTo(0, y + 0.5);
    ctx.lineTo(b.width, y + 0.5);
    ctx.stroke();
  }
}

/** Min/max of the bins under device column `col`, or null where nothing is decoded. */
function column(s: Scene, level: number, from: number, to: number): [number, number] | null {
  const first = Math.floor(from / level);
  const last = Math.max(first, Math.ceil(to / level) - 1);
  let min = 32767;
  let max = -32768;
  let any = false;
  for (let bin = first; bin <= last; bin++) {
    const mm = s.peaks.bin(level, bin);
    if (mm === null) continue;
    any = true;
    if (mm[0] < min) min = mm[0];
    if (mm[1] > max) max = mm[1];
  }
  return any ? [min, max] : null;
}

function drawWave(ctx: CanvasRenderingContext2D, b: Bands, s: Scene, p: Palette): void {
  const level = samplesPerBin(s.view.samplesPerPx, s.dpr);
  const mid = (b.waveTop + b.waveBottom) / 2;
  const half = ((b.waveBottom - b.waveTop) / 2) * 0.92;
  const cols = Math.ceil(b.width * s.dpr);
  const step = s.view.samplesPerPx / s.dpr;
  const w = 1 / s.dpr;
  for (let c = 0; c < cols; c++) {
    const from = s.view.start + c * step;
    if (from + step < 0 || from > s.frames) continue;
    const mm = column(s, level, Math.max(0, from), Math.min(s.frames, from + step));
    if (mm === null) continue;
    const x = c / s.dpr;
    const top = mid - (mm[1] / 32768) * half;
    const bottom = mid - (mm[0] / 32768) * half;
    ctx.fillStyle = p.waveOrig;
    ctx.fillRect(x, top, w, Math.max(w, bottom - top));
    const gTop = mid - Math.min(1, (mm[1] / 32768) * s.gain) * half;
    const gBottom = mid - Math.max(-1, (mm[0] / 32768) * s.gain) * half;
    ctx.fillStyle = p.waveProc;
    ctx.fillRect(x, gTop, w, Math.max(w, gBottom - gTop));
  }
}

function visibleLines(s: Scene, grid: Grid) {
  const from = s.view.start;
  const to = s.view.start + s.view.widthPx * s.view.samplesPerPx;
  return linesInRange(grid, s.sampleRate, Math.floor(from), Math.ceil(to));
}

function drawGrid(ctx: CanvasRenderingContext2D, b: Bands, s: Scene, grid: Grid, p: Palette): void {
  const beatPx = pxPerBeat(s.view, grid, s.sampleRate);
  for (const line of visibleLines(s, grid)) {
    if (line.kind === 'pulse' && beatPx < 5) continue;
    if (line.kind === 'group' && beatPx < 3) continue;
    const x = Math.round(sampleToX(s.view, line.sample)) + 0.5;
    const first = line.index === 0;
    ctx.strokeStyle = first
      ? p.accent
      : line.kind === 'bar'
        ? TEAL(0.8)
        : line.kind === 'group'
          ? TEAL(0.4)
          : TEAL(0.2);
    ctx.lineWidth = line.kind === 'bar' ? 1.5 : 1;
    ctx.beginPath();
    ctx.moveTo(x, b.waveTop);
    ctx.lineTo(x, b.waveBottom);
    ctx.stroke();
    if (first) {
      // The handle bar 1 is dragged by.
      ctx.fillStyle = p.accent;
      ctx.beginPath();
      ctx.moveTo(x - 6, b.waveTop);
      ctx.lineTo(x + 6, b.waveTop);
      ctx.lineTo(x, b.waveTop + 9);
      ctx.closePath();
      ctx.fill();
    }
  }
}

function drawGhost(
  ctx: CanvasRenderingContext2D,
  b: Bands,
  s: Scene,
  ghost: Grid,
  p: Palette,
): void {
  ctx.save();
  ctx.setLineDash([3, 4]);
  ctx.strokeStyle = p.fg2;
  ctx.lineWidth = 1;
  const beatPx = pxPerBeat(s.view, ghost, s.sampleRate);
  for (const line of visibleLines(s, ghost)) {
    if (line.kind !== 'bar' && beatPx < 8) continue;
    const x = Math.round(sampleToX(s.view, line.sample)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(x, b.waveTop);
    ctx.lineTo(x, b.waveBottom);
    ctx.stroke();
  }
  ctx.restore();
}

function drawRuler(ctx: CanvasRenderingContext2D, s: Scene, grid: Grid, p: Palette): void {
  const beatPx = pxPerBeat(s.view, grid, s.sampleRate);
  const barPx = beatPx * Math.max(1, grid.meter.beatsPerBar);
  const every = barPx >= 36 ? 1 : Math.pow(2, Math.ceil(Math.log2(36 / Math.max(barPx, 0.01))));
  ctx.font = '500 10.5px "JetBrains Mono", ui-monospace, monospace';
  ctx.textBaseline = 'middle';
  for (const line of visibleLines(s, grid)) {
    const x = Math.round(sampleToX(s.view, line.sample)) + 0.5;
    if (line.kind === 'bar') {
      const labelled = line.bar > 0 ? (line.bar - 1) % every === 0 : (-line.bar - 1) % every === 0;
      ctx.strokeStyle = TEAL(0.8);
      ctx.beginPath();
      ctx.moveTo(x, labelled ? 6 : 16);
      ctx.lineTo(x, RULER_H);
      ctx.stroke();
      if (labelled) {
        ctx.fillStyle = line.index === 0 ? p.accent : p.fg1;
        ctx.fillText(String(line.bar), x + 4, 12);
      }
    } else if (beatPx >= BEAT_LABEL_PX) {
      // Zoomed in: every pulse numbered, group starts in grey, the rest of a group in teal.
      ctx.strokeStyle = line.kind === 'group' ? TEAL(0.4) : TEAL(0.25);
      ctx.beginPath();
      ctx.moveTo(x, 19);
      ctx.lineTo(x, RULER_H);
      ctx.stroke();
      ctx.font = '500 9.5px "JetBrains Mono", ui-monospace, monospace';
      ctx.fillStyle = line.kind === 'group' ? p.fg2 : TEAL(0.75);
      ctx.fillText(rulerLabel(grid.meter, line.pulse), x + 3, 12);
      ctx.font = '500 10.5px "JetBrains Mono", ui-monospace, monospace';
    } else if (line.kind === 'group' && barPx >= 60) {
      ctx.strokeStyle = TEAL(0.4);
      ctx.beginPath();
      ctx.moveTo(x, 19);
      ctx.lineTo(x, RULER_H);
      ctx.stroke();
    }
  }
}

function drawLane(ctx: CanvasRenderingContext2D, b: Bands, s: Scene, grid: Grid, p: Palette): void {
  ctx.font = '500 10px "IBM Plex Sans", system-ui, sans-serif';
  ctx.fillStyle = p.fg2;
  ctx.textBaseline = 'top';
  ctx.fillText('Residual', 6, b.laneTop + 4);
  ctx.textAlign = 'right';
  ctx.textBaseline = 'bottom';
  ctx.fillText(`+ late · − early · scale ${LANE_SCALE_MS} ms`, b.width - 6, b.laneTop + LANE_H - 3);
  ctx.textAlign = 'left';
  const fit = s.fit;
  if (!fit || fit.residuals.length === 0) return;
  const mid = b.laneTop + LANE_H / 2;
  const scale = (LANE_H / 2 - 5) / LANE_SCALE_MS;
  const tone = (ms: number) => ({ ok: p.ok, warn: p.warn, err: p.err })[residualTone(ms)];
  ctx.fillStyle = p.line;
  ctx.fillRect(0, mid, b.width, 1);
  const spb = (60 * s.sampleRate) / grid.bpm;
  const beatPx = spb / s.view.samplesPerPx;
  const first = fit.header.firstLine;
  if (beatPx >= 20) {
    fit.residuals.forEach((r, k) => {
      if (Number.isNaN(r)) return;
      const x = sampleToX(s.view, grid.anchor + (first + k) * spb);
      if (x < -10 || x > b.width + 10) return;
      const h = Math.max(2, Math.min(LANE_SCALE_MS, Math.abs(r)) * scale);
      ctx.fillStyle = tone(r);
      ctx.fillRect(x - 2, r > 0 ? mid - h : mid + 1, 4, h);
    });
    return;
  }
  const bpb = Math.max(1, grid.meter.beatsPerBar);
  for (const [bar, worst] of barResiduals(fit.residuals, first, bpb)) {
    if (Number.isNaN(worst)) continue;
    const firstLine = bar > 0 ? (bar - 1) * bpb : bar * bpb;
    const x0 = sampleToX(s.view, grid.anchor + firstLine * spb);
    const x1 = x0 + bpb * beatPx;
    if (x1 < 0 || x0 > b.width) continue;
    const h = Math.max(2, Math.min(LANE_SCALE_MS, Math.abs(worst)) * scale);
    ctx.fillStyle = tone(worst);
    ctx.fillRect(x0 + 0.5, worst > 0 ? mid - h : mid + 1, Math.max(1, x1 - x0 - 1), h);
  }
}

function drawOverview(ctx: CanvasRenderingContext2D, b: Bands, s: Scene, p: Palette): void {
  if (s.frames <= 0) return;
  const top = b.overviewTop;
  const mid = top + OVERVIEW_H / 2;
  const half = OVERVIEW_H / 2 - 3;
  const spp = s.frames / b.width;
  const level = samplesPerBin(spp, s.dpr);
  ctx.fillStyle = p.waveOrig;
  for (let x = 0; x < b.width; x++) {
    const mm = column(s, level, x * spp, (x + 1) * spp);
    if (mm === null) continue;
    const t = mid - (mm[1] / 32768) * half;
    ctx.fillRect(x, t, 1, Math.max(1, mid - (mm[0] / 32768) * half - t));
  }
  const x0 = (s.view.start / s.frames) * b.width;
  const w = ((s.view.widthPx * s.view.samplesPerPx) / s.frames) * b.width;
  ctx.strokeStyle = p.teal;
  ctx.lineWidth = 1;
  ctx.strokeRect(Math.max(0, x0) + 0.5, top + 1.5, Math.max(2, w) - 1, OVERVIEW_H - 3);
}

export interface Overlay {
  view: View;
  frames: number;
  playhead: number | null;
  hover: number | null;
}

export function drawOverlay(ctx: CanvasRenderingContext2D, b: Bands, o: Overlay, p: Palette): void {
  ctx.clearRect(0, 0, b.width, b.height);
  if (o.hover !== null) {
    const x = Math.round(sampleToX(o.view, o.hover)) + 0.5;
    ctx.strokeStyle = p.fg2;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, RULER_H);
    ctx.lineTo(x, b.laneTop);
    ctx.stroke();
  }
  if (o.playhead !== null) {
    const x = Math.round(sampleToX(o.view, o.playhead)) + 0.5;
    ctx.strokeStyle = p.accent;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, b.overviewTop);
    ctx.stroke();
    if (o.frames > 0) {
      const ox = Math.round((o.playhead / o.frames) * b.width) + 0.5;
      ctx.beginPath();
      ctx.moveTo(ox, b.overviewTop);
      ctx.lineTo(ox, b.height);
      ctx.stroke();
    }
  }
}
