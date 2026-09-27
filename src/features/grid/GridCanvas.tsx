import { useEffect, useRef } from 'react';
import type { Grid } from '@/lib/ipc';
import {
  OVERVIEW_H,
  RULER_H,
  bands,
  drawOverlay,
  drawScene,
  readPalette,
  type Palette,
} from './draw';
import { placedBarOne } from './edit';
import { samplesPerBeat, sampleToX, snapped, xToSample, zoomAround, zoomLimits } from './geometry';
import { PeakTiles } from './peaks';
import { useTrack } from './store';
import { useView } from './viewStore';

/** Pixels per beat a track opens at, and where bar 1 sits then (share of the width). */
const OPEN_PX_PER_BEAT = 40;
const OPEN_BAR_ONE_AT = 0.15;
/** Pointer distance, in CSS pixels, at which the bar-1 handle is caught. */
const HANDLE_PX = 6;

type Drag =
  | { kind: 'handle'; sample: number }
  | { kind: 'pan'; x: number; start: number }
  | { kind: 'overview' };

/** The waveform, grid, residual lane and overview of the open track, drawn on two canvases. */
export function GridCanvas() {
  const box = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const tiles = useRef<PeakTiles | null>(null);
  const size = useRef({ width: 0, height: 0, dpr: 1 });
  const palette = useRef<Palette | null>(null);
  const pending = useRef<number | null>(null);
  const drag = useRef<Drag | null>(null);
  const heard = useRef({ position: 0, at: 0, playing: false });

  useEffect(() => {
    const scene = sceneRef.current;
    const overlay = overlayRef.current;
    const el = box.current;
    if (!scene || !overlay || !el) return;
    palette.current = readPalette();

    const limits = (): [number, number] => {
      const t = useTrack.getState();
      const rate = t.opened?.sampleRate ?? 44_100;
      return zoomLimits(rate, t.opened?.frames ?? rate * 600, size.current.width);
    };
    const playhead = (): number | null => {
      const h = heard.current;
      const rate = useTrack.getState().opened?.sampleRate ?? 44_100;
      if (!h.playing) return useTrack.getState().player.position > 0 ? h.position : null;
      return h.position + ((performance.now() - h.at) / 1000) * rate;
    };
    const shownGrid = (): Grid | null => {
      const g = useTrack.getState().grid;
      const d = drag.current;
      return g && d?.kind === 'handle' ? { ...g, anchor: d.sample } : g;
    };

    // The scene (waveform, grid, lane, overview) is drawn only when something in it changed;
    // the overlay (playhead, hover) on every frame asked for.
    let sceneDirty = true;
    const render = () => {
      pending.current = null;
      const t = useTrack.getState();
      const v = useView.getState();
      const { width, height, dpr } = size.current;
      const p = palette.current;
      if (!t.opened || !tiles.current || !p || width === 0) return;
      const b = bands(width, height);
      const sctx = scene.getContext('2d');
      const octx = overlay.getContext('2d');
      if (!sctx || !octx) return;
      sctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      octx.setTransform(dpr, 0, 0, dpr, 0, 0);
      if (sceneDirty) {
        sceneDirty = false;
        drawScene(
          sctx,
          b,
          {
            view: v.view,
            sampleRate: t.opened.sampleRate,
            frames: t.opened.frames,
            grid: shownGrid(),
            ghost: v.ghost ? t.opened.analysed : null,
            fit: t.fit,
            gain: Math.pow(10, t.opened.gain / 20),
            peaks: tiles.current,
            dpr,
          },
          p,
        );
      }
      const head = playhead();
      drawOverlay(
        octx,
        b,
        { view: v.view, frames: t.opened.frames, playhead: head, hover: v.hover },
        p,
      );
      if (heard.current.playing) {
        // Keep the playhead in view: page on when it passes 85 % of the width.
        if (head !== null && sampleToX(v.view, head) > width * 0.85) {
          v.setView({ ...v.view, start: head - width * 0.15 * v.view.samplesPerPx });
        }
        requestFrame();
      }
    };
    const requestFrame = () => {
      if (pending.current === null) pending.current = requestAnimationFrame(render);
    };
    const schedule = () => {
      sceneDirty = true;
      requestFrame();
    };

    const resize = () => {
      const r = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      size.current = { width: r.width, height: r.height, dpr };
      for (const c of [scene, overlay]) {
        c.width = Math.round(r.width * dpr);
        c.height = Math.round(r.height * dpr);
        c.style.width = `${r.width}px`;
        c.style.height = `${r.height}px`;
      }
      const v = useView.getState();
      v.setView({ ...v.view, widthPx: r.width });
      schedule();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(el);

    // A newly opened track: fresh tiles, and the view at beat zoom with bar 1 near the left.
    let openedFor: number | null = null;
    const onTrack = () => {
      const t = useTrack.getState();
      if (t.opened && t.fileId !== openedFor) {
        openedFor = t.fileId;
        tiles.current = new PeakTiles(
          (spb, first, n) => useTrack.getState().readBins(spb, first, n),
          schedule,
        );
        const g = t.opened.grid;
        const rate = t.opened.sampleRate;
        const spp = g ? samplesPerBeat(g, rate) / OPEN_PX_PER_BEAT : rate / 100;
        const width = size.current.width || 1000;
        useView.getState().setView({
          start: (g?.anchor ?? 0) - width * OPEN_BAR_ONE_AT * spp,
          samplesPerPx: spp,
          widthPx: width,
        });
      }
      schedule();
    };
    // Where the player was last heard: the playhead runs on from it between player events.
    const onPlayer = () => {
      const p = useTrack.getState().player;
      heard.current = { position: p.position, at: performance.now(), playing: p.playing };
      requestFrame();
    };
    const unTrack = useTrack.subscribe((t, prev) => {
      if (t.decoded !== prev.decoded) tiles.current?.refreshPartial();
      if (t.player !== prev.player) onPlayer();
      if (
        t.opened !== prev.opened ||
        t.fileId !== prev.fileId ||
        t.grid !== prev.grid ||
        t.fit !== prev.fit ||
        t.decoded !== prev.decoded
      ) {
        onTrack();
      }
    });
    // The hover line is on the overlay; the view and the ghost grid are in the scene.
    const unView = useView.subscribe((v, prev) =>
      v.view !== prev.view || v.ghost !== prev.ghost ? schedule() : requestFrame(),
    );
    onTrack();
    onPlayer();

    const at = (e: PointerEvent | WheelEvent | MouseEvent) => {
      const r = overlay.getBoundingClientRect();
      return { x: e.clientX - r.left, y: e.clientY - r.top };
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const v = useView.getState();
      const { x } = at(e);
      if (e.ctrlKey || e.metaKey) {
        const [min, max] = limits();
        v.setView(zoomAround(v.view, Math.exp(-e.deltaY * 0.01), x, min, max));
      } else {
        const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
        v.setView({ ...v.view, start: v.view.start + d * v.view.samplesPerPx });
      }
    };
    const onDown = (e: PointerEvent) => {
      const t = useTrack.getState();
      const v = useView.getState();
      if (!t.opened) return;
      const { x, y } = at(e);
      const b = bands(size.current.width, size.current.height);
      if (y >= b.overviewTop) {
        drag.current = { kind: 'overview' };
        centreOn((x / b.width) * t.opened.frames);
      } else if (y < RULER_H) {
        t.seek(xToSample(v.view, x));
      } else {
        const g = t.grid;
        if (g && Math.abs(sampleToX(v.view, g.anchor) - x) <= HANDLE_PX && y < b.laneTop) {
          drag.current = { kind: 'handle', sample: g.anchor };
        } else {
          drag.current = { kind: 'pan', x, start: v.view.start };
        }
      }
      overlay.setPointerCapture(e.pointerId);
    };
    const centreOn = (sample: number) => {
      const v = useView.getState();
      v.setView({ ...v.view, start: sample - (v.view.widthPx / 2) * v.view.samplesPerPx });
    };
    const onMove = (e: PointerEvent) => {
      const t = useTrack.getState();
      const v = useView.getState();
      if (!t.opened) return;
      const { x, y } = at(e);
      const b = bands(size.current.width, size.current.height);
      const d = drag.current;
      const sample = xToSample(v.view, x);
      if (d?.kind === 'handle') {
        drag.current = {
          kind: 'handle',
          sample: e.shiftKey ? Math.round(sample) : snapped(t.onsets, sample, t.opened.sampleRate),
        };
        schedule();
      } else if (d?.kind === 'pan') {
        v.setView({ ...v.view, start: d.start - (x - d.x) * v.view.samplesPerPx });
      } else if (d?.kind === 'overview') {
        centreOn((x / b.width) * t.opened.frames);
      }
      v.setHover(y > RULER_H && y < b.laneTop ? sample : null);
      overlay.style.cursor =
        d?.kind === 'handle' ||
        (t.grid &&
          Math.abs(sampleToX(v.view, t.grid.anchor) - x) <= HANDLE_PX &&
          y < b.laneTop &&
          y > RULER_H)
          ? 'ew-resize'
          : 'default';
    };
    const onCancel = () => {
      drag.current = null;
      schedule();
    };
    const onUp = () => {
      const d = drag.current;
      drag.current = null;
      const t = useTrack.getState();
      if (d?.kind === 'handle' && t.grid && d.sample !== t.grid.anchor) {
        t.edit(placedBarOne(t.edits.present, d.sample), { ...t.grid, anchor: d.sample });
      }
      schedule();
    };
    const onDouble = (e: MouseEvent) => {
      const t = useTrack.getState();
      const v = useView.getState();
      const { x, y } = at(e);
      const b = bands(size.current.width, size.current.height);
      if (!t.opened || !t.grid || y <= RULER_H || y >= b.laneTop) return;
      const sample = e.shiftKey
        ? Math.round(xToSample(v.view, x))
        : snapped(t.onsets, xToSample(v.view, x), t.opened.sampleRate);
      t.edit(placedBarOne(t.edits.present, sample), { ...t.grid, anchor: sample });
    };
    const onLeave = () => useView.getState().setHover(null);
    overlay.addEventListener('wheel', onWheel, { passive: false });
    overlay.addEventListener('pointerdown', onDown);
    overlay.addEventListener('pointermove', onMove);
    overlay.addEventListener('pointerup', onUp);
    overlay.addEventListener('pointercancel', onCancel);
    overlay.addEventListener('dblclick', onDouble);
    overlay.addEventListener('pointerleave', onLeave);
    return () => {
      observer.disconnect();
      unTrack();
      unView();
      if (pending.current !== null) cancelAnimationFrame(pending.current);
      pending.current = null;
      overlay.removeEventListener('wheel', onWheel);
      overlay.removeEventListener('pointerdown', onDown);
      overlay.removeEventListener('pointermove', onMove);
      overlay.removeEventListener('pointerup', onUp);
      overlay.removeEventListener('pointercancel', onCancel);
      overlay.removeEventListener('dblclick', onDouble);
      overlay.removeEventListener('pointerleave', onLeave);
    };
  }, []);

  return (
    <div
      ref={box}
      className="relative min-h-0 flex-1 overflow-hidden bg-bg-0"
      role="img"
      aria-label="Waveform with bar ruler, beat grid, residual lane and overview"
      style={{ minHeight: RULER_H + OVERVIEW_H + 160 }}
    >
      <canvas ref={sceneRef} className="absolute inset-0" />
      <canvas ref={overlayRef} className="absolute inset-0 touch-none" />
    </div>
  );
}
