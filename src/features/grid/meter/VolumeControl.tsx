import { useCallback, useEffect, useRef, useState } from 'react';
import { Volume1, Volume2, VolumeX } from 'lucide-react';
import { cn } from '@/lib/utils';
import { VOLUME_MAX_DB, VOLUME_MIN_DB, volumeAt, volumePos, volumeText } from '@/lib/volume';
import { useMonitor } from '@/state/monitor';
import { TRANSPORT_FOLD } from '../fold';
import { Btn } from '../ui';

/** The slider's travel, CSS pixels. */
export const SLIDER_W = 96;
/** Within this many pixels of the thumb, a press drags it rather than jumping to the pointer. */
const THUMB_CATCH_PX = 7;
/** Shift+drag moves the volume this much slower. */
const FINE = 0.1;
const ICON = 'size-3.5 shrink-0';

function SpeakerIcon({ db, muted }: { db: number | null; muted: boolean }) {
  if (muted || db === null) return <VolumeX className={ICON} aria-hidden />;
  return db < -20 ? (
    <Volume1 className={ICON} aria-hidden />
  ) : (
    <Volume2 className={ICON} aria-hidden />
  );
}

function MuteButton() {
  const db = useMonitor((s) => s.db);
  const muted = useMonitor((s) => s.muted);
  const toggleMute = useMonitor((s) => s.toggleMute);
  return (
    <Btn
      pressed={muted}
      onClick={toggleMute}
      aria-label="Mute"
      title={muted ? 'Unmute' : 'Mute'}
      className="px-2"
    >
      <SpeakerIcon db={db} muted={muted} />
    </Btn>
  );
}

/**
 * The monitor volume slider: 0 dB at the right end, -60 dB then off at the left; the level shows
 * on hover and while dragging (always, in the popover). Shift+drag moves it ten times slower;
 * arrows step 1 dB, Page Up/Down 6 dB, Home is off, End 0 dB.
 */
function VolumeSlider({ inPopover = false }: { inPopover?: boolean }) {
  const db = useMonitor((s) => s.db);
  const muted = useMonitor((s) => s.muted);
  const setDb = useMonitor((s) => s.setDb);
  const step = useMonitor((s) => s.step);
  const track = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; pos: number; fine: boolean } | null>(null);
  const [hover, setHover] = useState(false);
  const [dragging, setDragging] = useState(false);
  const alwaysShow = inPopover;
  // Opened from its icon, the popover's slider takes the keys: arrows step, Esc closes.
  useEffect(() => {
    if (inPopover) track.current?.focus();
  }, [inPopover]);
  const pos = volumePos(db);
  const text = muted ? `Muted · ${volumeText(db)}` : volumeText(db);

  const onPointerDown = (e: React.PointerEvent) => {
    const el = track.current;
    if (!el) return;
    // Pressed, not focused, like the buttons: the window keeps its keys.
    e.preventDefault();
    el.setPointerCapture?.(e.pointerId);
    const r = el.getBoundingClientRect();
    const x = e.clientX - r.left;
    let start = pos;
    if (Math.abs(x - pos * r.width) > THUMB_CATCH_PX) {
      start = x / r.width;
      setDb(volumeAt(start));
    }
    drag.current = { x: e.clientX, pos: start, fine: e.shiftKey };
    setDragging(true);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    const el = track.current;
    if (!d || !el) return;
    const width = el.getBoundingClientRect().width || SLIDER_W;
    if (e.shiftKey !== d.fine) {
      // Shift pressed or let go mid-drag: carry on from here at the new speed.
      drag.current = { x: e.clientX, pos: volumePos(useMonitor.getState().db), fine: e.shiftKey };
      return;
    }
    setDb(volumeAt(d.pos + ((e.clientX - d.x) / width) * (d.fine ? FINE : 1)));
  };
  const onPointerUp = () => {
    drag.current = null;
    setDragging(false);
  };
  const onKeyDown = (e: React.KeyboardEvent) => {
    const keys: Record<string, () => void> = {
      ArrowRight: () => step(1),
      ArrowUp: () => step(1),
      ArrowLeft: () => step(-1),
      ArrowDown: () => step(-1),
      PageUp: () => step(6),
      PageDown: () => step(-6),
      Home: () => setDb(null),
      End: () => setDb(VOLUME_MAX_DB),
    };
    const act = keys[e.key];
    if (!act || e.metaKey || e.ctrlKey || e.altKey) return;
    // The slider's, not the grid view's (arrows pan there).
    e.preventDefault();
    e.stopPropagation();
    act();
  };

  return (
    <div
      ref={track}
      role="slider"
      tabIndex={0}
      aria-label="Volume"
      aria-valuemin={VOLUME_MIN_DB}
      aria-valuemax={VOLUME_MAX_DB}
      aria-valuenow={db ?? VOLUME_MIN_DB}
      aria-valuetext={text}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onKeyDown={onKeyDown}
      className="group relative flex h-[30px] shrink-0 cursor-default touch-none items-center rounded-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2"
      style={{ width: SLIDER_W }}
    >
      <div className="relative h-1 w-full rounded-full bg-bg-3">
        <div
          className={cn('absolute inset-y-0 left-0 rounded-full', muted ? 'bg-fg-2' : 'bg-fg-1')}
          style={{ width: `${pos * 100}%` }}
        />
        <div
          className={cn(
            'absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full',
            muted ? 'bg-fg-2' : 'bg-fg-0',
          )}
          style={{ left: `${pos * 100}%` }}
        />
      </div>
      {(alwaysShow || hover || dragging) && (
        <span
          aria-hidden
          className={cn(
            'pointer-events-none whitespace-nowrap font-mono text-[11px] tabular-nums text-fg-0',
            alwaysShow
              ? 'absolute left-full ml-2.5'
              : 'absolute bottom-[30px] z-20 -translate-x-1/2 rounded-[5px] border border-line bg-bg-2 px-1.5 py-0.5 shadow-[0_4px_12px_rgba(0,0,0,.4)]',
          )}
          style={alwaysShow ? undefined : { left: `${pos * 100}%` }}
        >
          {text}
        </span>
      )}
    </div>
  );
}

/**
 * The monitor volume beside play/pause: the speaker icon mutes, the slider sets the level. In a
 * window narrower than 1000 px, the icon opens both in a popover.
 */
export function VolumeControl({ narrow }: { narrow: boolean }) {
  if (narrow) return <VolumePopover />;
  // A row too tight for the slider (a 1000-1200 px window beside the rail) folds it the same way.
  return (
    <>
      <div
        className={cn('flex items-center gap-1', TRANSPORT_FOLD.tight)}
        role="group"
        aria-label="Monitor volume"
      >
        <MuteButton />
        <VolumeSlider />
      </div>
      <div className={TRANSPORT_FOLD.tightOnly}>
        <VolumePopover />
      </div>
    </>
  );
}

function VolumePopover() {
  const [open, setOpen] = useState(false);
  const db = useMonitor((s) => s.db);
  const muted = useMonitor((s) => s.muted);
  const box = useRef<HTMLDivElement>(null);
  /** The popover was opened from the keyboard (Tab to the speaker button, then Space or Enter). */
  const fromKeyboard = useRef(false);
  /**
   * Closes the popover. Opened from the keyboard, Esc gives focus back to the speaker button.
   * Opened with a click, focus goes back to the page, where Space plays (a focused button would
   * take it); and a press outside never moves focus.
   */
  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus && fromKeyboard.current)
      box.current?.querySelector<HTMLButtonElement>('button')?.focus();
  }, []);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (box.current && e.target instanceof Node && !box.current.contains(e.target)) close(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [open, close]);
  return (
    <div ref={box} className="relative">
      <Btn
        pressed={open}
        onClick={(e) => {
          // A click has a count; Space or Enter on the focused button has none.
          fromKeyboard.current = e.detail === 0;
          setOpen(!open);
        }}
        aria-label="Volume"
        aria-haspopup="dialog"
        aria-expanded={open}
        title={`Volume · ${muted ? 'Muted' : volumeText(db)}`}
        className="px-2"
      >
        <SpeakerIcon db={db} muted={muted} />
      </Btn>
      {open && (
        <div
          role="dialog"
          aria-label="Monitor volume"
          className="absolute bottom-[38px] left-0 z-20 flex items-center gap-1 rounded-lg border border-line bg-bg-2 py-1 pl-1 pr-3 shadow-[0_8px_24px_rgba(0,0,0,.45)]"
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              close(true);
            }
          }}
        >
          <MuteButton />
          <VolumeSlider inPopover />
          {/* Room for the level beside the slider. */}
          <span className="w-[86px]" />
        </div>
      )}
    </div>
  );
}
