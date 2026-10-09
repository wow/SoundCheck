import { useEffect, useState } from 'react';
import { useSelector } from '@xstate/react';
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


/** Keys for the review loop: arrows move, N jumps to the next row that needs a look. */
function useKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The grid view handles its own keys while it is open.
      if (useTrack.getState().fileId !== null) return;
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
  const notice = useLibrary((s) => s.notice);
  const review = useLibrary((s) => s.order.filter((id) => s.rows[id]?.state === 'needsReview').length);
  const state = useSelector(pipeline(), (s) => s.value);
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
              Analysis could not start: {aborted.message}
            </div>
          )}
          {hasRows ? <Table /> : <EmptyState />}
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
                className="text-fg-2 hover:text-fg-0"
                title="Removes every track from this list. The files are not touched."
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
      <Footer cancelling={state === 'cancelling'} />
      {dropping && (
        <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-[14px] border-2 border-dashed border-accent bg-[rgba(12,14,18,.75)] text-lg font-semibold">
          Drop to add
        </div>
      )}
    </div>
  );
}
