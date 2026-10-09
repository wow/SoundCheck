import { useEffect, useMemo, useRef } from 'react';
import type { MeterFrame } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { useLibrary } from '@/state/library';
import { useSettings } from '@/state/settings';
import { useTrack } from '../store';
import { NO_PEAK, barLevel, holdLevel, peakInput, smoothed } from './ballistics';
import {
  type StripPalette,
  type StripScene,
  STRIP_W,
  STRIP_W_NARROW,
  drawStrip,
  readStripPalette,
  stripLayout,
} from './drawMeter';
import {
  type Caption,
  SCALE_MAX,
  SCALE_MIN,
  SIDE_LABEL,
  type Side,
  caption as captionOf,
  captionText,
  readingText,
} from './scale';

/** What changes rarely: drawn into the canvas and the readouts, but set by React. */
interface Fixed {
  caption: Caption;
  target: number | null;
  ceiling: number | null;
  over: boolean;
  narrow: boolean;
}

const UNIT = { peak: 'dBTP', loudness: 'LUFS' } as const;
const MONO_NOTE = 'mono sum: this long track plays and meters as mono (left and right averaged)';

/** One meter's readings for its side of a frame. */
function readings(frame: MeterFrame, side: Side): { peak: number | null; loudness: number | null } {
  return side === 'in'
    ? { peak: frame.inPeak, loudness: frame.inMomentary }
    : { peak: frame.outPeak, loudness: frame.outMomentary };
}

/** Writes `value` to an attribute or the text only when it changed. */
function put(el: HTMLElement | null, attr: string | null, value: string): void {
  if (!el) return;
  if (attr === null) {
    if (el.textContent !== value) el.textContent = value;
  } else if (el.getAttribute(attr) !== value) {
    el.setAttribute(attr, value);
  }
}

/** Sets a `role="meter"` element's value; none reads as the bottom of the scale. */
function putMeter(el: HTMLElement | null, label: string, db: number | null, unit: string): void {
  const has = db !== null && Number.isFinite(db);
  put(el, 'aria-label', label);
  put(el, 'aria-valuenow', String(has ? Math.min(SCALE_MAX, Math.max(SCALE_MIN, db)) : SCALE_MIN));
  put(el, 'aria-valuetext', has ? `${readingText(db, unit === UNIT.peak)} ${unit}` : 'no signal');
}

/**
 * A live meter beside the waveform: IN (the original) on the left, OUT (at the planned gain)
 * on the right. The bar is the true peak with a peak-hold tick, the arrow the momentary
 * loudness; OUT also draws the target (dashed) and the ceiling. Drawn on animation frames from
 * the 30 Hz readings, never through a React render; stopped, it shows the track's numbers.
 */
export function MeterStrip({ side, narrow }: { side: Side; narrow: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const peakRef = useRef<HTMLSpanElement>(null);
  const loudRef = useRef<HTMLSpanElement>(null);
  const captionRef = useRef<HTMLSpanElement>(null);
  const redraw = useRef<() => void>(() => {});

  const fileId = useTrack((s) => s.fileId);
  const gain = useTrack((s) => s.opened?.gain ?? 0);
  const heard = useTrack((s) => (s.listen.version === 'original') === (side === 'in'));
  const over = useTrack((s) => s.overs[side]);
  const clearOver = useTrack((s) => s.clearOver);
  const analysis = useLibrary((s) => (fileId === null ? undefined : s.rows[fileId]?.analysis));
  const mode = useSettings((s) => s.mode);
  const target = useSettings((s) => s.target);
  const ceiling = useSettings((s) => s.ceiling);
  const label = SIDE_LABEL[side];

  const cap = useMemo(() => captionOf(side, analysis, mode, gain), [side, analysis, mode, gain]);
  const fixed = useRef<Fixed>({ caption: cap, target: null, ceiling: null, over, narrow });
  useEffect(() => {
    fixed.current = {
      caption: cap,
      // No loudness measured (silence): no target to read against.
      target: side === 'out' && cap.loudness !== null ? target : null,
      ceiling: side === 'out' ? ceiling : null,
      over,
      narrow,
    };
    redraw.current();
  }, [cap, side, target, ceiling, over, narrow]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const el = box.current;
    if (!canvas || !el) return;
    let palette: StripPalette | null = null;
    let size = { width: 0, height: 0, dpr: 1 };
    let peak = NO_PEAK;
    let loudness: number | null = null;
    let shown: number | null = null;
    let frame: MeterFrame | null = null;
    let playing = false;
    let last = performance.now();
    let pending: number | null = null;

    const writeReadouts = (now: number) => {
      const f = fixed.current;
      const name = SIDE_LABEL[side];
      let peakDb: number | null;
      let loudDb: number | null;
      let tip: string;
      // A long stereo track is held and played as mono: say so, quietly.
      const mono = playing && frame?.folded === true;
      const of = mono ? ', mono sum' : '';
      if (playing) {
        peakDb = holdLevel(peak, now);
        loudDb = shown;
        putMeter(peakRef.current, `${name} true peak${of}`, peakDb, UNIT.peak);
        putMeter(loudRef.current, `${name} momentary loudness${of}`, loudDb, UNIT.loudness);
        tip = `${name} · peak ${readingText(peakDb, true)} dBTP · momentary ${readingText(loudDb, false)} LUFS`;
        if (mono) tip += ` · ${MONO_NOTE}`;
      } else {
        peakDb = f.caption.truePeak;
        loudDb = f.caption.loudness;
        putMeter(peakRef.current, `${name} true peak of the track`, peakDb, UNIT.peak);
        putMeter(loudRef.current, `${name} ${f.caption.stat}`, loudDb, UNIT.loudness);
        tip = `${name} · ${captionText(f.caption)}`;
      }
      put(peakRef.current, null, readingText(peakDb, true));
      put(loudRef.current, null, readingText(loudDb, false));
      put(captionRef.current, null, captionText(f.caption));
      if (captionRef.current) captionRef.current.hidden = playing;
      if (f.narrow) put(el, 'title', tip);
      else if (mono) put(el, 'title', `${name} · ${MONO_NOTE}`);
      else el.removeAttribute('title');
    };

    const render = (now: number) => {
      pending = null;
      const dt = now - last;
      last = now;
      shown = smoothed(shown, loudness, dt);
      writeReadouts(now);
      const ctx = canvas.getContext('2d');
      if (!ctx || size.width === 0 || !palette) return;
      const started = import.meta.env.DEV ? performance.now() : 0;
      const f = fixed.current;
      const scene: StripScene = {
        bar: playing ? barLevel(peak, now) : null,
        hold: playing ? holdLevel(peak, now) : null,
        momentary: playing ? shown : null,
        target: f.target,
        ceiling: f.ceiling,
        over: f.over,
      };
      ctx.setTransform(size.dpr, 0, 0, size.dpr, 0, 0);
      drawStrip(ctx, stripLayout(side, size.width, size.height), scene, palette);
      if (import.meta.env.DEV) measure(started);
      if (playing) request();
    };
    const request = () => {
      if (pending === null) pending = requestAnimationFrame(render);
    };
    // Readouts are text: written at once, so a change shows even before the next frame.
    redraw.current = () => {
      writeReadouts(performance.now());
      request();
    };

    const resize = () => {
      const r = el.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      size = { width: r.width, height: r.height, dpr };
      canvas.width = Math.round(r.width * dpr);
      canvas.height = Math.round(r.height * dpr);
      canvas.style.width = `${r.width}px`;
      canvas.style.height = `${r.height}px`;
      request();
    };
    palette = readStripPalette();
    const observer = new ResizeObserver(resize);
    observer.observe(el);

    const onTrack = () => {
      const t = useTrack.getState();
      if (t.player.playing === playing && t.meter === frame) return;
      const now = performance.now();
      if (!t.player.playing) {
        // Stopped: the bars empty and the readouts go back to the track's numbers.
        peak = NO_PEAK;
        loudness = null;
        shown = null;
      } else if (t.meter && t.meter !== frame) {
        const r = readings(t.meter, side);
        peak = peakInput(peak, r.peak, now);
        loudness = r.loudness;
      } else if (!t.meter) {
        // Just after a seek: the peak falls on, the loudness waits for 400 ms of audio.
        loudness = null;
      }
      if (!playing && t.player.playing) last = now;
      playing = t.player.playing;
      frame = t.meter;
      if (!playing) writeReadouts(now);
      request();
    };
    const unTrack = useTrack.subscribe(onTrack);
    onTrack();
    redraw.current();
    return () => {
      observer.disconnect();
      unTrack();
      redraw.current = () => {};
      if (pending !== null) cancelAnimationFrame(pending);
    };
  }, [side]);

  return (
    <div
      ref={box}
      role="group"
      aria-label={`${label} meter${side === 'in' ? ', the original' : ', at the planned gain'}`}
      data-heard={heard}
      onPointerDown={() => over && clearOver(side)}
      className={cn(
        'relative shrink-0 select-none border-line bg-bg-1',
        side === 'in' ? 'border-r' : 'border-l',
        !heard && 'opacity-[.45]',
      )}
      style={{ width: narrow ? STRIP_W_NARROW : STRIP_W }}
    >
      <canvas ref={canvasRef} aria-hidden className="absolute inset-0" />
      <div
        className={
          narrow
            ? 'sr-only'
            : 'absolute inset-x-0 top-0 flex h-[26px] flex-col items-center justify-center font-mono text-[9px] leading-[11px] tracking-[-0.03em] tabular-nums'
        }
      >
        <span
          ref={peakRef}
          role="meter"
          aria-valuemin={SCALE_MIN}
          aria-valuemax={SCALE_MAX}
          className={over ? 'text-err' : 'text-meter-tp'}
        />
        <span
          ref={loudRef}
          role="meter"
          aria-valuemin={SCALE_MIN}
          aria-valuemax={SCALE_MAX}
          className="text-meter-lufs"
        />
      </div>
      {!narrow && (
        // Stopped, the empty well carries the track's numbers, read from the bottom up.
        <span
          ref={captionRef}
          aria-hidden
          className={cn(
            'absolute bottom-[32px] w-[10px] rotate-180 whitespace-nowrap bg-bg-0 py-1 font-mono text-[9px] leading-[10px] text-fg-1 [writing-mode:vertical-rl]',
            side === 'in' ? 'right-[3px]' : 'left-[3px]',
          )}
        />
      )}
      <div className="absolute inset-x-0 bottom-0 flex h-[28px] items-center justify-center">
        {over ? (
          <button
            type="button"
            onClick={() => clearOver(side)}
            aria-label={`${label} peaked over ${side === 'in' ? '0 dBTP' : 'the ceiling'}; clear`}
            title={`Peaked over ${side === 'in' ? '0 dBTP' : 'the ceiling'} · click to clear`}
            className={cn(
              'rounded-[3px] font-sans font-semibold text-err focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2',
              narrow ? 'size-2 bg-err' : 'px-0.5 text-[10px]',
            )}
          >
            {narrow ? null : label}
          </button>
        ) : (
          !narrow && (
            <span aria-hidden className="font-sans text-[10px] font-semibold text-fg-2">
              {label}
            </span>
          )
        )}
      </div>
    </div>
  );
}

/** Dev builds: how long a strip takes to draw, as `sc:meter-draw` performance measures. */
let measures = 0;
function measure(started: number): void {
  if (++measures % 600 === 0) performance.clearMeasures('sc:meter-draw');
  performance.measure?.('sc:meter-draw', { start: started, end: performance.now() });
}
