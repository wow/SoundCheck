import { useEffect, useRef, useState } from 'react';
import { ChevronDown, Search, SlidersHorizontal } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { NARROW, useMediaQuery } from '@/lib/media';
import { chooseFiles, chooseFolders } from '@/lib/platform';
import { cn } from '@/lib/utils';
import { counts, staleIds, useLibrary, type Filter } from '@/state/library';
import { usePanels } from '@/state/panels';
import { presetLabel, useSettings } from '@/state/settings';
import { addPaths, analyseStale } from '@/features/pipeline/actions';

/** The filter chips; `short` is the label in a narrow window. */
const CHIPS: { id: Filter; label: string; short?: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'review', label: 'Needs review', short: 'Review' },
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

/**
 * The header: filter chips, the name filter, Add and Analyse. In a window narrower than
 * 1000 px it folds: no wordmark, shorter chip labels, the name filter behind an icon (opened in
 * the chips' place), one Add menu, and the settings button.
 */
export function Toolbar({ busy }: { busy: boolean }) {
  const c = useLibrary(useShallow(counts));
  const filter = useLibrary((s) => s.filter);
  const setFilter = useLibrary((s) => s.setFilter);
  const query = useLibrary((s) => s.query);
  const range = useSettings((s) => s.bpmRange);
  const stale = useLibrary((s) => staleIds(s, range).length);
  const narrow = useMediaQuery(NARROW);
  const searching = usePanels((p) => p.search) || query !== '';
  return (
    <header className="flex h-[52px] shrink-0 items-center gap-2 border-b border-line bg-bg-1 px-4 xl:gap-3">
      {!narrow && (
        <div className="flex items-center gap-2 pr-3">
          <Logo />
          <span className="hidden text-sm font-semibold tracking-tight xl:inline">SoundCheck</span>
        </div>
      )}
      {!(narrow && searching) && (
        <nav aria-label="Filter tracks" className="flex items-center gap-1">
          {CHIPS.map((chip) => (
            <button
              key={chip.id}
              type="button"
              aria-pressed={filter === chip.id}
              onClick={() => setFilter(chip.id)}
              title={narrow && chip.short ? chip.label : undefined}
              className={cn(
                'inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-md text-[12.5px] font-medium',
                narrow ? 'px-1.5' : 'px-2 xl:px-2.5',
                filter === chip.id ? 'bg-bg-3 text-fg-0' : 'text-fg-1 hover:bg-bg-2',
              )}
            >
              {narrow && chip.short ? chip.short : chip.label}
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
      )}
      {!(narrow && searching) && <div className="flex-1" />}
      <NameFilter narrow={narrow} open={!narrow || searching} />
      {narrow ? (
        <AddMenu />
      ) : (
        <>
          <button
            type="button"
            className={cn(button, 'border-line text-fg-0 hover:bg-bg-2')}
            onClick={() => void chooseFolders().then(addPaths)}
          >
            Add folder
          </button>
          <button
            type="button"
            className={cn(button, 'border-line text-fg-0 hover:bg-bg-2')}
            onClick={() => void chooseFiles().then(addPaths)}
          >
            Add tracks
          </button>
        </>
      )}
      <button
        type="button"
        className={cn(button, 'border-transparent bg-accent text-bg-0')}
        disabled={busy || stale === 0}
        onClick={analyseStale}
        title={stale === 0 ? 'Every track is analysed with the current settings' : undefined}
      >
        Analyse{!busy && stale > 0 ? ` ${stale}` : ''}
      </button>
      <SettingsButton />
    </header>
  );
}

/**
 * The name filter. Folded (narrow window, nothing typed) it is an icon; opened, by the icon or
 * Cmd+F, it takes the chips' place and closes again when left empty.
 */
function NameFilter({ narrow, open }: { narrow: boolean; open: boolean }) {
  const query = useLibrary((s) => s.query);
  const setQuery = useLibrary((s) => s.setQuery);
  const setSearch = usePanels((p) => p.setSearch);
  const input = useRef<HTMLInputElement>(null);
  const wasOpen = useRef(open);
  useEffect(() => {
    // Opened from the icon or Cmd+F: straight into the field.
    if (narrow && open && !wasOpen.current) input.current?.focus();
    wasOpen.current = open;
  }, [narrow, open]);
  if (!open) {
    return (
      <button
        type="button"
        aria-label="Filter by name"
        title="Filter by name · ⌘F"
        onClick={() => setSearch(true)}
        className={cn(button, 'border-line px-2 text-fg-0 hover:bg-bg-2')}
      >
        <Search className="size-3.5" aria-hidden="true" />
      </button>
    );
  }
  return (
    <label
      className={cn(
        'flex h-[30px] min-w-[120px] shrink items-center gap-2 rounded-[7px] border border-line bg-bg-0 px-2.5',
        narrow ? 'flex-1' : 'w-40 xl:w-56',
      )}
    >
      <span className="sr-only">Filter by name</span>
      <input
        ref={input}
        id={SEARCH_ID}
        type="search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onBlur={() => {
          if (useLibrary.getState().query === '') setSearch(false);
        }}
        placeholder="Filter by name"
        className="min-w-0 flex-1 bg-transparent text-[13px] text-fg-0 outline-none placeholder:text-fg-2"
      />
      <span className="hidden rounded border border-line px-1 font-mono text-[10px] leading-[15px] text-fg-2 xl:inline">
        ⌘F
      </span>
    </label>
  );
}

/** Add folder and Add tracks as one menu, in a narrow window. */
function AddMenu() {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    box.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    const onDown = (e: PointerEvent) => {
      if (e.target instanceof Node && !box.current?.contains(e.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', onDown);
    return () => document.removeEventListener('pointerdown', onDown);
  }, [open]);
  const choose = (pick: () => Promise<string[]>) => {
    setOpen(false);
    void pick().then(addPaths);
  };
  return (
    <div ref={box} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className={cn(button, 'border-line text-fg-0 hover:bg-bg-2')}
      >
        Add
        <ChevronDown className="size-3.5" aria-hidden="true" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Add"
          className="absolute right-0 top-[34px] z-30 flex w-44 flex-col rounded-lg border border-line bg-bg-2 p-1 shadow-[0_8px_24px_rgba(0,0,0,.45)]"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setOpen(false);
            if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
              e.preventDefault();
              const items = [...(box.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
              const at = items.indexOf(document.activeElement as HTMLElement);
              items[(at + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
            }
            e.stopPropagation();
          }}
        >
          <button type="button" role="menuitem" className={MENU_ITEM} onClick={() => choose(chooseFolders)}>
            Add folder…
          </button>
          <button type="button" role="menuitem" className={MENU_ITEM} onClick={() => choose(chooseFiles)}>
            Add tracks…
          </button>
        </div>
      )}
    </div>
  );
}

const MENU_ITEM =
  'flex h-8 items-center rounded-md px-2.5 text-left text-[13px] text-fg-0 hover:bg-bg-3 focus-visible:bg-bg-3 focus-visible:outline-none';

/**
 * In a narrow window, where the settings rail is a drawer: the button that opens it, showing
 * the setting that matters most, what the batch is levelled to (`DJ · −11 LUFS`).
 */
function SettingsButton() {
  const narrow = useMediaQuery(NARROW);
  const open = usePanels((p) => p.settings);
  const setOpen = usePanels((p) => p.setSettings);
  const settings = useSettings();
  if (!narrow) return null;
  const name = settings.mode === 'dj' ? 'DJ' : presetLabel(settings);
  const target = settings.target.toFixed(1).replace('-', '\u2212');
  return (
    <button
      type="button"
      aria-haspopup="dialog"
      aria-expanded={open}
      onClick={() => setOpen(!open)}
      title="Settings · ⌘,"
      className={cn(button, 'border-line text-fg-0 hover:bg-bg-2', open && 'bg-bg-3')}
    >
      <SlidersHorizontal className="size-3.5 shrink-0" aria-hidden="true" />
      <span className="sr-only">Settings: </span>
      <span>
        {name} · <span className="font-mono tabular-nums">{target}</span> LUFS
      </span>
    </button>
  );
}
