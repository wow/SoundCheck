import { useCallback, useEffect, useState } from 'react';
import { ownsKey } from '@/lib/focus';
import {
  back,
  barOneHere,
  beatIsOne,
  confirmAndNext,
  jumpTo,
  nextInQueue,
  nudge,
  pan,
  playPause,
  resetGrid,
  tap,
  toBarOne,
  toggleClick,
  zoom,
} from './actions';
import { CursorReadout } from './CursorReadout';
import { barOf, samplesPerBeat } from './geometry';
import { GridCanvas } from './GridCanvas';
import { GridHeader } from './GridHeader';
import { GridRail } from './GridRail';
import { GridToolbar } from './GridToolbar';
import { gridKey } from './keys';
import { useTrack } from './store';
import { useView } from './viewStore';

/** The grid view's keys, while it is open. */
function useGridKeys(toggleMeter: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (ownsKey(e.target, e.key) || e.repeat) return;
      const action = gridKey(e);
      if (!action) return;
      e.preventDefault();
      const t = useTrack.getState();
      switch (action.type) {
        case 'confirmNext':
          return void confirmAndNext();
        case 'nextReview':
          return void nextInQueue();
        case 'back':
          return void back();
        case 'playPause':
          return playPause();
        case 'toBarOne':
          return toBarOne();
        case 'click':
          return toggleClick();
        case 'ghost':
          return useView.getState().toggleGhost();
        case 'barOneHere':
          return barOneHere(action.free);
        case 'nudge':
          return nudge(action.direction, action.unit);
        case 'beatOne':
          return beatIsOne(action.beat);
        case 'tap':
          return void tap();
        case 'meter':
          return toggleMeter();
        case 'reset':
          return resetGrid();
        case 'undo':
          return t.undo();
        case 'redo':
          return t.redo();
        case 'pan':
          return pan(action.bars);
        case 'zoom':
          return zoom(action.factor);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [toggleMeter]);
}

/** Opening, analysing again, decoding, or why something failed: one line above the waveform. */
function StatusLine() {
  const phase = useTrack((s) => s.phase);
  const analysing = useTrack((s) => s.analysing);
  const error = useTrack((s) => s.error);
  const decoded = useTrack((s) => s.decoded);
  const done = useTrack((s) => s.decodeDone);
  const frames = useTrack((s) => s.opened?.frames ?? 0);
  const playerError = useTrack((s) => s.player.error);
  let text: string | null = null;
  let tone = 'text-fg-2';
  if (phase === 'opening')
    text =
      analysing === null
        ? 'Opening…'
        : `Analysing again for the grid view… ${Math.round(analysing * 100)} %`;
  else if (phase === 'failed' || (error && done && decoded < frames)) {
    text = error ?? 'The track could not be opened.';
    tone = 'text-err';
  } else if (playerError) {
    text = playerError;
    tone = 'text-warn';
  } else if (!done && frames > 0) text = `Loading audio… ${Math.round((decoded / frames) * 100)} %`;
  if (!text) return null;
  return (
    <div
      role="status"
      className={`shrink-0 border-b border-line bg-bg-1 px-4 py-1.5 text-[12px] ${tone}`}
    >
      {text}
    </div>
  );
}

function DriftStrip() {
  const grid = useTrack((s) => s.grid);
  const worst = useTrack((s) => s.fit?.header.worstLine ?? null);
  const rate = useTrack((s) => s.opened?.sampleRate ?? 44_100);
  if (!grid || grid.verdict !== 'drifts') return null;
  const bar = worst === null ? null : barOf(worst, Math.max(1, grid.meter.beatsPerBar));
  const ppm = `${grid.driftPpm > 0 ? '+' : ''}${grid.driftPpm.toFixed(0)} ppm`;
  return (
    <div className="flex h-14 shrink-0 items-center gap-3.5 border-b border-line bg-[rgba(248,113,113,.06)] px-[18px]">
      <div className="flex min-w-0 flex-col">
        <span className="text-[13px] font-medium text-err">
          Drifts: max {grid.residualMaxMs.toFixed(0)} ms{bar !== null ? ` at bar ${bar}` : ''},
          trend {ppm}
        </span>
        <span className="truncate text-[12px] text-fg-2">
          A static grid will not fit the whole track. It is exported static from bar 1 and flagged
          Drifts.
        </span>
      </div>
      {worst !== null && bar !== null && (
        <button
          type="button"
          onClick={() => jumpTo(grid.anchor + worst * samplesPerBeat(grid, rate))}
          className="ml-auto h-[30px] shrink-0 rounded-[7px] border border-line px-3 text-[13px] font-medium text-fg-1 hover:bg-bg-2"
        >
          Jump to bar {bar}
        </button>
      )}
    </div>
  );
}

/** The grid view: one track's waveform and grid, its fixes and the click. */
export function GridView() {
  const [meterOpen, setMeterOpen] = useState(false);
  const toggleMeter = useCallback(() => setMeterOpen((open) => !open), []);
  useGridKeys(toggleMeter);
  return (
    <div className="flex h-full flex-col bg-bg-0 text-fg-0">
      <GridHeader />
      <div className="flex min-h-0 flex-1">
        <main className="flex min-w-0 flex-1 flex-col">
          <StatusLine />
          <div className="relative flex min-h-0 flex-1 flex-col">
            <GridCanvas />
            <CursorReadout />
          </div>
          <GridToolbar meterOpen={meterOpen} setMeterOpen={setMeterOpen} />
          <DriftStrip />
        </main>
        <GridRail />
      </div>
    </div>
  );
}
