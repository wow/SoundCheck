import { useShallow } from 'zustand/react/shallow';
import { chooseFiles, chooseFolders } from '@/lib/platform';
import { cn } from '@/lib/utils';
import { counts, staleIds, useLibrary, type Filter } from '@/state/library';
import { useSettings } from '@/state/settings';
import { addPaths, analyseStale, clearList } from '@/features/pipeline/actions';

const CHIPS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'review', label: 'Needs review' },
  { id: 'short', label: 'Short' },
  { id: 'skipped', label: 'Skipped' },
  { id: 'done', label: 'Done' },
];

export const SEARCH_ID = 'track-filter';

function Logo() {
  return (
    <span className="inline-flex size-5 items-center justify-center rounded-md bg-accent" aria-hidden="true">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#0c0e12" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M3 12h2l2-6 3 12 3-9 2 6 2-3h4" />
      </svg>
    </span>
  );
}

const button =
  'inline-flex h-[30px] items-center gap-1.5 whitespace-nowrap rounded-[7px] border px-3 text-[13px] font-medium disabled:cursor-default disabled:opacity-45';

/** The header: filter chips, the name filter, Add and Analyse. */
export function Toolbar({ busy }: { busy: boolean }) {
  const c = useLibrary(useShallow(counts));
  const filter = useLibrary((s) => s.filter);
  const setFilter = useLibrary((s) => s.setFilter);
  const query = useLibrary((s) => s.query);
  const setQuery = useLibrary((s) => s.setQuery);
  const range = useSettings((s) => s.bpmRange);
  const stale = useLibrary((s) => staleIds(s, range).length);
  const empty = c.all === 0;
  return (
    <header className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line bg-bg-1 px-4">
      <div className="flex items-center gap-2 pr-3">
        <Logo />
        <span className="text-sm font-semibold tracking-tight">SoundCheck</span>
      </div>
      <nav aria-label="Filter tracks" className="flex items-center gap-1">
        {CHIPS.map((chip) => (
          <button
            key={chip.id}
            type="button"
            aria-pressed={filter === chip.id}
            onClick={() => setFilter(chip.id)}
            className={cn(
              'inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] font-medium',
              filter === chip.id ? 'bg-bg-3 text-fg-0' : 'text-fg-1 hover:bg-bg-2',
            )}
          >
            {chip.label}
            <span
              className={cn(
                'font-mono text-[11px]',
                chip.id === 'review' && c.review > 0 ? 'text-warn' : 'text-fg-2',
                chip.id === 'short' && c.short > 0 && 'text-accent',
              )}
            >
              {c[chip.id]}
            </span>
          </button>
        ))}
      </nav>
      <div className="flex-1" />
      <label className="flex h-[30px] w-56 items-center gap-2 rounded-[7px] border border-line bg-bg-0 px-2.5">
        <span className="sr-only">Filter by name</span>
        <input
          id={SEARCH_ID}
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter by name"
          className="min-w-0 flex-1 bg-transparent text-[13px] text-fg-0 outline-none placeholder:text-fg-2"
        />
        <span className="rounded border border-line px-1 font-mono text-[10px] leading-[15px] text-fg-2">⌘F</span>
      </label>
      <button
        type="button"
        className={cn(button, 'border-transparent text-fg-2 hover:text-fg-0')}
        disabled={empty}
        onClick={() => void clearList()}
        title="Removes every track from this list. The files are not touched."
      >
        Clear list
      </button>
      <button type="button" className={cn(button, 'border-line text-fg-0 hover:bg-bg-2')} onClick={() => void chooseFolders().then(addPaths)}>
        Add folder
      </button>
      <button type="button" className={cn(button, 'border-line text-fg-0 hover:bg-bg-2')} onClick={() => void chooseFiles().then(addPaths)}>
        Add tracks
      </button>
      <button
        type="button"
        className={cn(button, 'border-transparent bg-accent text-bg-0')}
        disabled={busy || stale === 0}
        onClick={analyseStale}
        title={stale === 0 ? 'Every track is analysed with the current settings' : undefined}
      >
        Analyse{!busy && stale > 0 ? ` ${stale}` : ''}
      </button>
    </header>
  );
}
