import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ChevronLeft,
  ChevronRight,
  Crosshair,
  Layers2,
  Maximize2,
  Metronome,
  Pause,
  Play,
  RotateCcw,
  SkipBack,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import {
  barOneHere,
  beatIsOne,
  chooseMeter,
  fitWhole,
  nudge,
  octave,
  playPause,
  resetGrid,
  tap,
  toBarOne,
  toggleClick,
  typeBpm,
  zoom,
} from './actions';
import { samplesPerBeat } from './geometry';
import { playText } from './readout';
import { meterText, pickerMeters } from './meters';
import { edited, useTrack } from './store';
import { Btn, Kbd } from './ui';
import { useView } from './viewStore';

/**
 * Hidden when the fix row is narrower than everything it holds (1080 px): key hints stay in
 * the tooltips and the rail's key legend, the octave buttons in the fit card.
 */
const COMPACT_HIDDEN = '@max-[1080px]:hidden';
/** Hidden when a row is narrower still (a narrow window): labels whose control keeps an icon. */
const NARROW_HIDDEN = '@max-[820px]:hidden';
const ICON = 'size-3.5 shrink-0';

/** `40`, or `0.3` when the whole of a long track is in view. */
function formatPxPerBeat(px: number): string {
  return px >= 10 ? String(Math.round(px)) : px.toFixed(1);
}

function Group({
  children,
  label,
  className,
}: {
  children: React.ReactNode;
  label: string;
  className?: string;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className={cn(
        'flex h-[30px] shrink-0 items-center gap-1 whitespace-nowrap border-r border-line px-2.5 @max-[900px]:px-1.5',
        className,
      )}
    >
      {children}
    </div>
  );
}

/** Two rows under the waveform: listening and looking, then every fix. */
export function GridToolbar({
  meterOpen,
  setMeterOpen,
}: {
  meterOpen: boolean;
  setMeterOpen: (open: boolean) => void;
}) {
  return (
    <>
      <TransportBar />
      <FixBar meterOpen={meterOpen} setMeterOpen={setMeterOpen} />
    </>
  );
}

function TransportBar() {
  const grid = useTrack((s) => s.grid);
  const opened = useTrack((s) => s.opened);
  const playing = useTrack((s) => s.player.playing);
  const click = useTrack((s) => s.click);
  const isEdited = useTrack(edited);
  // Only the zoom: the view's start moves every frame while it follows the playhead.
  const samplesPerPx = useView((s) => s.view.samplesPerPx);
  const ghost = useView((s) => s.ghost);
  const toggleGhost = useView((s) => s.toggleGhost);
  const rate = opened?.sampleRate ?? 44_100;
  return (
    <div className="@container flex h-10 shrink-0 items-center border-t border-line bg-bg-1">
      <Group label="Transport">
        <Btn onClick={toBarOne} aria-label="To bar 1" title="To bar 1 · Home" className="px-2">
          <SkipBack className={ICON} aria-hidden />
        </Btn>
        <Btn hint="Space" onClick={playPause} aria-label={playing ? 'Pause' : 'Play'}>
          {playing ? (
            <Pause className={ICON} fill="currentColor" aria-hidden />
          ) : (
            <Play className={ICON} fill="currentColor" aria-hidden />
          )}
        </Btn>
        <PlayClock />
      </Group>
      <Group label="Listen" className="border-r-0">
        <Btn hint="C" pressed={click} onClick={toggleClick}>
          <Metronome className={ICON} aria-hidden />
          Click
        </Btn>
        <Btn
          hint="A"
          pressed={ghost}
          onClick={toggleGhost}
          disabled={!isEdited}
          aria-label="Before / after"
          title="The analysed grid, dashed, under yours"
        >
          <Layers2 className={ICON} aria-hidden />
          <span className="@max-[780px]:hidden">Before / after</span>
        </Btn>
      </Group>
      <div className="flex-1" />
      <Group label="Zoom" className="border-r-0">
        <Btn
          onClick={() => zoom(1 / 1.5)}
          aria-label="Zoom out"
          title="Zoom out · −"
          className="px-2"
        >
          <ZoomOut className={ICON} aria-hidden />
        </Btn>
        <Btn onClick={() => zoom(1.5)} aria-label="Zoom in" title="Zoom in · +" className="px-2">
          <ZoomIn className={ICON} aria-hidden />
        </Btn>
        <Btn onClick={fitWhole} aria-label="Whole track" title="Whole track" className="px-2">
          <Maximize2 className={ICON} aria-hidden />
        </Btn>
        <span className="w-[88px] text-right font-mono text-[11.5px] tabular-nums text-fg-1 @max-[780px]:hidden">
          {grid ? `${formatPxPerBeat(samplesPerBeat(grid, rate) / samplesPerPx)} px / beat` : '–'}
        </span>
      </Group>
    </div>
  );
}

/** Where the player is, written straight into the DOM on every player event. */
function PlayClock() {
  const clock = useRef<HTMLSpanElement>(null);
  const bar = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    const write = () => {
      const text = playText(useTrack.getState());
      if (clock.current) clock.current.textContent = text.clock;
      if (bar.current) bar.current.textContent = text.bar;
    };
    write();
    return useTrack.subscribe(write);
  }, []);
  return (
    <span className="px-1 font-mono text-[12px] tabular-nums text-fg-1">
      <span ref={clock} />
      <span className="px-1.5 text-fg-2">|</span>bar <span ref={bar} />
    </span>
  );
}

function nudgeUnit(e: React.MouseEvent): 'ms' | '10ms' | 'beat' {
  return e.altKey ? 'beat' : e.shiftKey ? '10ms' : 'ms';
}

function FixBar({
  meterOpen,
  setMeterOpen,
}: {
  meterOpen: boolean;
  setMeterOpen: (open: boolean) => void;
}) {
  const grid = useTrack((s) => s.grid);
  const isEdited = useTrack(edited);
  const closeMeter = useCallback(() => setMeterOpen(false), [setMeterOpen]);
  return (
    <div className="@container flex h-10 shrink-0 items-center border-y border-line bg-bg-1">
      <Group label="Bar 1">
        <Btn
          hint="D"
          hintClassName={COMPACT_HIDDEN}
          onClick={(e) => barOneHere(e.shiftKey)}
          aria-label="Beat 1 here"
          title="Bar 1 at the pointer, on the nearest kick (Shift: exactly there) · D"
        >
          <Crosshair className={ICON} aria-hidden />
          <span className={NARROW_HIDDEN}>Beat 1 here</span>
        </Btn>
        <Btn
          onClick={(e) => nudge(-1, nudgeUnit(e))}
          aria-label="Nudge bar 1 earlier"
          title="Bar 1 1 ms earlier (Shift 10 ms, Alt 1 beat) · ,"
          className="px-2"
        >
          <ChevronLeft className={ICON} aria-hidden />
        </Btn>
        <Btn
          onClick={(e) => nudge(1, nudgeUnit(e))}
          aria-label="Nudge bar 1 later"
          title="Bar 1 1 ms later (Shift 10 ms, Alt 1 beat) · ."
          className="px-2"
        >
          <ChevronRight className={ICON} aria-hidden />
        </Btn>
      </Group>
      <Group label="Which beat is beat 1">
        <span className={cn('pr-1 text-[11px] text-fg-2', NARROW_HIDDEN)}>beat 1 =</span>
        {grid && <BeatOneButtons grouping={grid.meter.grouping} />}
      </Group>
      <Group label="Tempo">
        <BpmField bpm={grid?.bpm ?? null} />
        <Btn
          hint="T"
          hintClassName={COMPACT_HIDDEN}
          onClick={() => tap()}
          title="Tap the tempo · T"
        >
          Tap
        </Btn>
        <Btn
          onClick={() => octave(-1)}
          aria-label="Half the tempo"
          className={cn('px-2.5', COMPACT_HIDDEN)}
        >
          ½
        </Btn>
        <Btn
          onClick={() => octave(1)}
          aria-label="Double the tempo"
          className={cn('px-2.5', COMPACT_HIDDEN)}
        >
          ×2
        </Btn>
      </Group>
      <Group label="Meter" className="border-r-0">
        <div className="relative">
          <Btn
            hint="M"
            hintClassName={COMPACT_HIDDEN}
            pressed={meterOpen}
            onClick={() => setMeterOpen(!meterOpen)}
            aria-haspopup="menu"
            aria-label={`Meter ${grid ? meterText(grid.meter) : ''}`.trim()}
            title="Choose the meter · M"
          >
            <span className={NARROW_HIDDEN}>Meter </span>
            {/* The time signature: the grouped beat buttons beside it show the grouping. */}
            <span className="font-mono text-[12px]">
              {grid ? meterText({ ...grid.meter, grouping: [grid.meter.beatsPerBar] }) : '–'}
            </span>
          </Btn>
          {meterOpen && grid && <MeterMenu close={closeMeter} />}
        </div>
      </Group>
      <div className="flex-1" />
      <Group label="Reset" className="border-r-0 border-l">
        <Btn
          hint="R"
          hintClassName={COMPACT_HIDDEN}
          onClick={resetGrid}
          disabled={!isEdited}
          aria-label="Reset"
          title="Back to the analysed grid · R"
        >
          <RotateCcw className={ICON} aria-hidden />
          <span className={COMPACT_HIDDEN}>Reset</span>
        </Btn>
      </Group>
    </div>
  );
}

/**
 * One button per pulse, spaced by the meter's grouping (9/8 2+2+2+3 reads `12 34 56 789`);
 * keys 1-9 do the same.
 */
function BeatOneButtons({ grouping }: { grouping: number[] }) {
  const grouped = grouping.some((g) => g > 1);
  let pulse = 0;
  return (
    <div className={cn('flex', grouped ? 'gap-[7px]' : 'gap-[3px]')}>
      {grouping.map((size, g) => (
        <div key={g} className="flex gap-[2px]">
          {Array.from({ length: size }, () => {
            const beat = ++pulse;
            return (
              <button
                key={beat}
                type="button"
                onClick={() => beatIsOne(beat)}
                aria-label={`Pulse ${beat} is beat 1`}
                aria-pressed={beat === 1}
                className={cn(
                  'h-[26px] w-[22px] rounded-md border font-mono text-[12px] font-semibold',
                  'focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2',
                  beat === 1
                    ? 'border-accent bg-[rgba(245,185,66,.15)] text-accent'
                    : 'border-line text-fg-1 hover:bg-bg-2',
                )}
              >
                {beat}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

function BpmField({ bpm }: { bpm: number | null }) {
  // While typing, the field holds the draft; otherwise it shows the grid's tempo.
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? (bpm === null ? '' : bpm.toFixed(2));
  return (
    <label className="inline-flex h-7 w-[92px] items-center gap-1.5 rounded-md border border-line bg-bg-1 px-2 focus-within:border-accent-2 focus-within:outline focus-within:outline-1 focus-within:outline-accent-2">
      <span className="text-[11px] text-fg-2">BPM</span>
      <input
        value={text}
        inputMode="decimal"
        aria-label="BPM"
        className="w-full bg-transparent font-mono text-[12.5px] tabular-nums text-fg-0 outline-none"
        onFocus={() => setDraft(text)}
        onBlur={() => setDraft(null)}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            typeBpm(Number(text.replace(',', '.')));
            (e.target as HTMLInputElement).blur();
          } else if (e.key === 'Escape') {
            (e.target as HTMLInputElement).blur();
          }
          e.stopPropagation();
        }}
      />
    </label>
  );
}

/**
 * The meter picker: the runner-up first, then the other meters. Arrow keys move between them;
 * `Esc` or a press outside (other than on the Meter button, which toggles it) closes it.
 */
function MeterMenu({ close }: { close: () => void }) {
  const grid = useTrack((s) => s.grid);
  const menu = useRef<HTMLDivElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => first.current?.focus(), []);
  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      const inside = menu.current?.parentElement;
      if (inside && e.target instanceof Node && !inside.contains(e.target)) close();
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [close]);
  if (!grid) return null;
  const meters = pickerMeters(grid.meter, grid.meterRunnerUp);
  return (
    <div
      ref={menu}
      role="menu"
      className="absolute bottom-[38px] left-0 z-20 flex w-[220px] flex-col rounded-lg border border-line bg-bg-2 p-1 shadow-[0_8px_24px_rgba(0,0,0,.45)]"
      onKeyDown={(e) => {
        if (e.key === 'Escape') close();
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          const items = [
            ...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []),
          ];
          const at = items.indexOf(document.activeElement as HTMLElement);
          const step = e.key === 'ArrowDown' ? 1 : -1;
          items[(at + step + items.length) % items.length]?.focus();
        }
        e.stopPropagation();
      }}
    >
      {meters.map((m, i) => (
        <button
          key={meterText(m)}
          ref={i === 0 ? first : undefined}
          role="menuitem"
          type="button"
          className="flex h-8 items-center justify-between rounded-md px-2.5 text-left font-mono text-[12.5px] text-fg-0 hover:bg-bg-3 focus-visible:bg-bg-3 focus-visible:outline-none"
          onClick={() => {
            chooseMeter(m);
            close();
          }}
        >
          {meterText(m)}
          {i === 0 && grid.meterRunnerUp && (
            <span className="font-sans text-[11px] text-fg-2">runner-up</span>
          )}
        </button>
      ))}
      <div className="px-2.5 pb-1 pt-1.5 text-[11px] text-fg-2">
        <Kbd>Esc</Kbd> closes
      </div>
    </div>
  );
}
