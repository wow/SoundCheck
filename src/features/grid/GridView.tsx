import { useCallback, useEffect, useState } from 'react';
import { Drawer } from '@/components/Drawer';
import { ownsKey } from '@/lib/focus';
import { NARROW, useMediaQuery } from '@/lib/media';
import { usePanels } from '@/state/panels';
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
  stepVolume,
  switchVersion,
  tap,
  toBarOne,
  toggleClick,
  toggleMatch,
  zoom,
} from './actions';
import { CursorReadout } from './CursorReadout';
import { barOf, samplesPerBeat } from './geometry';
import { GridCanvas } from './GridCanvas';
import { GridHeader } from './GridHeader';
import { FitStrip } from './FitStrip';
import { GridDetails, GridRail } from './GridRail';
import { GridToolbar } from './GridToolbar';
import { type GridKey, gridKey } from './keys';
import { MeterStrip } from './meter/MeterStrip';
import { useTrack } from './store';
import { useView } from './viewStore';

const DRAWER_KEYS = new Set<GridKey['type']>([
  'details',
  'undo',
  'redo',
  'playPause',
  'click',
  'version',
  'match',
  'volume',
]);

/** The grid view's keys, while it is open. */
function useGridKeys(toggleMeter: () => void) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (ownsKey(e.target, e.key)) return;
      const action = gridKey(e);
      // A held key repeats only the volume.
      if (!action || (e.repeat && action.type !== 'volume')) return;
      // While the details drawer is open, only I (closes it), undo, redo and what is heard
      // (play, the click, the version, the volume) reach the view; Esc closes the drawer rather
      // than leaving.
      const panels = usePanels.getState();
      if (panels.details) {
        if (action.type === 'back') return panels.setDetails(false);
        if (!DRAWER_KEYS.has(action.type)) return;
      }
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
        case 'details':
          // Only a narrow window folds the rail into a drawer.
          if (window.matchMedia?.(NARROW).matches) panels.setDetails(!panels.details);
          return;
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
        case 'version':
          return switchVersion();
        case 'match':
          return toggleMatch();
        case 'volume':
          return stepVolume(action.step);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [toggleMeter]);
}

/**
 * Opening, analysing again, decoding, or why something failed: one line above the waveform. In
 * a narrow window, where the rail that shows them is folded away, also a failed save (with
 * Save again) and why the grid shown is not the edit's.
 */
function StatusLine({ narrow }: { narrow: boolean }) {
  const phase = useTrack((s) => s.phase);
  const saveError = useTrack((s) => s.saveError);
  const saving = useTrack((s) => s.saving);
  const refitNote = useTrack((s) => s.refitNote);
  const retrySave = useTrack((s) => s.retrySave);
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
  } else if (narrow && saveError && !saving) {
    return (
      <div
        role="alert"
        className="flex shrink-0 items-center gap-3 border-b border-line bg-bg-1 px-4 py-1 text-[12px] text-err"
      >
        <span className="min-w-0 flex-1 truncate">Not saved: {saveError}</span>
        <button
          type="button"
          onClick={retrySave}
          className="h-6 shrink-0 rounded-md border border-line px-2 text-[12px] font-medium text-fg-1 hover:bg-bg-2"
        >
          Save again
        </button>
      </div>
    );
  } else if (narrow && refitNote) {
    text = refitNote;
    tone = 'text-warn';
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
  const narrow = useMediaQuery(NARROW);
  const details = usePanels((p) => p.details);
  const setDetails = usePanels((p) => p.setDetails);
  // Widened past the narrow layout, or left: the rail is back (or gone), so no drawer.
  useEffect(() => {
    if (!narrow) setDetails(false);
  }, [narrow, setDetails]);
  useEffect(() => () => setDetails(false), [setDetails]);
  return (
    <div className="flex h-full flex-col bg-bg-0 text-fg-0">
      <GridHeader />
      <div className="flex min-h-0 flex-1">
        <main className="flex min-w-0 flex-1 flex-col">
          {narrow && <FitStrip />}
          <StatusLine narrow={narrow} />
          <div className="flex min-h-0 flex-1">
            <MeterStrip side="in" narrow={narrow} />
            <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
              <GridCanvas />
              <CursorReadout />
            </div>
            <MeterStrip side="out" narrow={narrow} />
          </div>
          <GridToolbar meterOpen={meterOpen} setMeterOpen={setMeterOpen} />
          <DriftStrip />
        </main>
        {!narrow && <GridRail />}
      </div>
      {narrow && details && (
        <Drawer label="Grid details" focus="panel" onClose={() => setDetails(false)}>
          <GridDetails />
        </Drawer>
      )}
    </div>
  );
}
