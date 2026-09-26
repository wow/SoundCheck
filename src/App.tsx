import { useEffect, useState } from 'react';
import { useSelector } from '@xstate/react';
import { listenForDrops } from '@/lib/platform';
import { useLibrary } from '@/state/library';
import { EmptyState } from '@/features/library/EmptyState';
import { startTrackListPersistence } from '@/features/library/remember';
import { Footer } from '@/features/library/Footer';
import { Table } from '@/features/library/Table';
import { SEARCH_ID, Toolbar } from '@/features/library/Toolbar';
import { addPaths, clearList, pipeline } from '@/features/pipeline/actions';
import { Rail } from '@/features/settings/Rail';
import { startSettingsSync } from '@/features/settings/sync';

function isTyping(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLSelectElement || target instanceof HTMLTextAreaElement;
}

/** Keys for the review loop: arrows move, N jumps to the next row that needs a look. */
function useKeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const lib = useLibrary.getState();
      if (e.metaKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        document.getElementById(SEARCH_ID)?.focus();
        return;
      }
      if (isTyping(e.target)) {
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
      } else if (e.key === 'Escape') {
        lib.setFilter('all');
        lib.setQuery('');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

/** The main screen: toolbar, the table (or the drop target), the settings rail and the footer. */
export default function App() {
  const hasRows = useLibrary((s) => s.order.length > 0);
  const aborted = useLibrary((s) => s.aborted);
  const notice = useLibrary((s) => s.notice);
  const review = useLibrary((s) => s.order.filter((id) => s.rows[id]?.state === 'needsReview').length);
  const state = useSelector(pipeline(), (s) => s.value);
  const [dropping, setDropping] = useState(false);

  useKeys();
  useEffect(() => startSettingsSync(), []);
  useEffect(() => startTrackListPersistence(), []);
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
              <span>
                <kbd className="font-mono">↑↓</kbd> row
              </span>
              <span>
                <kbd className="font-mono">N</kbd> next needs review
              </span>
              <span>
                <kbd className="font-mono">⌘F</kbd> filter
              </span>
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
        <Rail />
      </div>
      <Footer cancelling={state === 'cancelling'} />
      {dropping && (
        <div className="pointer-events-none absolute inset-2 flex items-center justify-center rounded-[14px] border-2 border-dashed border-accent bg-[rgba(12,14,18,.75)] text-lg font-semibold">
          Drop to add
        </div>
      )}
    </div>
  );
}
