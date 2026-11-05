import { useEffect, useState } from 'react';
import { useSelector } from '@xstate/react';
import { X } from 'lucide-react';
import { errorCopy } from '@/lib/ipc';
import { keepFocusOnClick, ownsKey } from '@/lib/focus';
import { listenForDrops } from '@/lib/platform';
import { useLibrary } from '@/state/library';
import { EmptyState } from '@/features/library/EmptyState';
import { startTrackListPersistence } from '@/features/library/remember';
import { Footer } from '@/features/library/Footer';
import { Table } from '@/features/library/Table';
import { SEARCH_ID, Toolbar } from '@/features/library/Toolbar';
import { addPaths, clearList, pipeline } from '@/features/pipeline/actions';
import { Rail, Settings } from '@/features/settings/Rail';
import { Drawer } from '@/components/Drawer';
import { NARROW, useMediaQuery } from '@/lib/media';
import { usePanels } from '@/state/panels';
import { openInGridView } from '@/features/grid/actions';
import { GridView } from '@/features/grid/GridView';
import { useTrack } from '@/features/grid/store';
import { startMonitorSync } from '@/features/settings/monitorSync';
import { startSettingsSync } from '@/features/settings/sync';
import { ExportConfirm } from '@/features/export/ExportConfirm';
import { ExportDetail } from '@/features/export/ExportDetail';
import { useExportFlow } from '@/features/export/flow';
import { startRecoveryWatch } from '@/state/recovery';


/** Keys for the review loop: arrows move, N jumps to the next row that needs a look. */
function useKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The grid view handles its own keys while it is open.
      if (useTrack.getState().fileId !== null) return;
      // The confirm step, while open, has the keys to itself.
      if (useExportFlow.getState().phase !== 'closed') return;
      const panels = usePanels.getState();
      if (e.metaKey && e.key === ',') {
        // Settings: the drawer in a narrow window, else the rail's first control.
        e.preventDefault();
        if (window.matchMedia?.(NARROW).matches) panels.setSettings(!panels.settings);
        else document.querySelector<HTMLElement>('[data-settings-rail] button')?.focus();
        return;
      }
      // The settings drawer, while open, has the keys to itself.
      if (panels.settings) return;
      const lib = useLibrary.getState();
      if (e.metaKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        // In a narrow window the field is folded into an icon: opening it focuses it.
        panels.setSearch(true);
        document.getElementById(SEARCH_ID)?.focus();
        return;
      }
      if (ownsKey(e.target, e.key)) {
        if (e.key === 'Escape' && e.target instanceof HTMLInputElement && e.target.id === SEARCH_ID) {
          lib.setQuery('');
          e.target.blur();
        }
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        lib.moveSelection(e.key === 'ArrowDown' ? 1 : -1);
      } else if (e.key === 'n' || e.key === 'N') {
        lib.selectNextReview();
      } else if (e.key === 'Enter' && lib.selected !== null) {
        e.preventDefault();
        openInGridView(lib.selected);
      } else if (e.key === 'Escape') {
        lib.setFilter('all');
        lib.setQuery('');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

export default function App() {
  const inGrid = useTrack((s) => s.fileId !== null);
  // For the app's lifetime, not the table's: the table unmounts while the grid view is open, and
  // restoring the list again on its return would reset the selection and every row.
  useKeys();
  useEffect(() => startSettingsSync(), []);
  useEffect(() => startMonitorSync(), []);
  useEffect(() => startTrackListPersistence(), []);
  useEffect(() => startRecoveryWatch(), []);
  return (
    <div className="contents" onMouseDownCapture={keepFocusOnClick}>
      {inGrid ? <GridView /> : <Library />}
    </div>
  );
}

/** The table's keys, under it (left out in a narrow window, where they do not fit). */
function KeyHints() {
  return (
    <>
      <span>
        <kbd className="font-mono">↑↓</kbd> row
      </span>
      <span>
        <kbd className="font-mono">⏎</kbd> open grid
      </span>
      <span>
        <kbd className="font-mono">N</kbd> next needs review
      </span>
      <span>
        <kbd className="font-mono">⌘F</kbd> filter
      </span>
      <span>
        <kbd className="font-mono">⌘,</kbd> settings
      </span>
    </>
  );
}

/** The main screen: toolbar, the table (or the drop target), the settings rail and the footer. */
function Library() {
  const hasRows = useLibrary((s) => s.order.length > 0);
  const aborted = useLibrary((s) => s.aborted);
  const abortedKind = useLibrary((s) => s.abortedKind);
  const artefactsFailed = useLibrary((s) => s.artefactsFailed);
  const exported = useLibrary((s) => (s.lastExport?.written ?? 0) > 0);
  const notice = useLibrary((s) => s.notice);
  const info = useLibrary((s) => s.info);
  const setInfo = useLibrary((s) => s.setInfo);
  const review = useLibrary((s) => s.order.filter((id) => s.rows[id]?.state === 'needsReview').length);
  const state = useSelector(pipeline(), (s) => s.value);
  const exportRunning =
    state === 'exporting' || state === 'cancellingExport' || state === 'windingDown';
  const [dropping, setDropping] = useState(false);
  const narrow = useMediaQuery(NARROW);
  const settingsOpen = usePanels((p) => p.settings);
  const setSettingsOpen = usePanels((p) => p.setSettings);
  // Widened past the narrow layout: the rail is back beside the table, so no drawer.
  useEffect(() => {
    if (!narrow) setSettingsOpen(false);
  }, [narrow, setSettingsOpen]);

  useEffect(
    () =>
      listenForDrops(
        (paths) => void addPaths(paths),
        (hover) => setDropping(hover === 'over'),
      ),
    [],
  );

  return (
    <div className="relative flex h-full flex-col bg-bg-0 text-fg-0">
      <Toolbar busy={state !== 'idle'} />
      <div className="flex min-h-0 flex-1">
        <main className="flex min-w-0 flex-1 flex-col">
          {notice && (
            <div role="alert" className="border-b border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.10)] px-4 py-2 text-[12.5px] text-warn">
              {notice}
            </div>
          )}
          {aborted && (
            <div role="alert" className="border-b border-[rgba(248,113,113,.45)] bg-[rgba(248,113,113,.10)] px-4 py-2 text-[12.5px] text-err">
              {abortedKind === 'export' ? 'Export' : 'Analysis'} could not start: {errorCopy(aborted)}
            </div>
          )}
          {artefactsFailed && (
            <div role="alert" className="border-b border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.10)] px-4 py-2 text-[12.5px] text-warn">
              The rekordbox XML and the grid report could not be written: {errorCopy(artefactsFailed)}
              {exported && ' The exported files are fine.'}
            </div>
          )}
          {info && (
            <div role="status" className="flex items-center gap-2 border-b border-line bg-bg-2 px-4 py-2 text-[12.5px] text-fg-1">
              <span className="min-w-0 flex-1">{info}</span>
              <button type="button" aria-label="Dismiss" onClick={() => setInfo(null)} className="shrink-0 text-fg-2 hover:text-fg-0">
                <X className="size-3.5" aria-hidden="true" />
              </button>
            </div>
          )}
          {hasRows ? <Table /> : <EmptyState />}
          {hasRows && <ExportDetail />}
          {hasRows && (
            <div className="flex h-8 shrink-0 items-center gap-4 border-t border-line px-4 text-[11.5px] text-fg-2">
              {!narrow && <KeyHints />}
              <div className="flex-1" />
              {review > 0 && (
                <span className="text-warn">
                  {review === 1 ? '1 track needs' : `${review} tracks need`} a look before processing
                </span>
              )}
              <button
                type="button"
                onClick={() => void clearList()}
                disabled={exportRunning}
                className="text-fg-2 hover:text-fg-0 disabled:opacity-45 disabled:hover:text-fg-2"
                title={
                  exportRunning
                    ? 'The list stays while an export runs. Cancel it first.'
                    : 'Removes every track from this list. The files are not touched.'
                }
              >
                Clear list
              </button>
            </div>
          )}
        </main>
        {!narrow && <Rail />}
      </div>
      {narrow && settingsOpen && (
        <Drawer
          label="Settings"
          onClose={() => setSettingsOpen(false)}
          returnFocus={() => document.querySelector<HTMLElement>('[data-settings-rail] button')}
        >
          <Settings />
        </Drawer>
      )}
      <Footer
        cancelling={state === 'cancelling' || state === 'cancellingExport'}
        windingDown={state === 'windingDown'}
        idle={state === 'idle'}
      />
      <ExportConfirm />
      {dropping && (
        <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-[14px] border-2 border-dashed border-accent bg-[rgba(12,14,18,.75)] text-lg font-semibold">
          Drop to add
        </div>
      )}
    </div>
  );
}
