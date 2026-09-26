import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { cn } from '@/lib/utils';
import { useLibrary, visibleIds } from '@/state/library';
import { useSettings } from '@/state/settings';
import { COLUMNS, NAME_MIN, rowDomId } from './columns';
import {
  ActionCell,
  BpmCell,
  LoudnessCell,
  NameCell,
  SpecCell,
  StatusPill,
  TpCell,
} from './cells';

/** Row height in px; rows are fixed-height so only the visible ones are rendered. */
export const ROW_HEIGHT = 44;
const OVERSCAN = 8;
/** Rows rendered before the viewport has been measured. */
const MIN_ROWS = 20;

function Cell({ width, children, className }: { width?: number; children: ReactNode; className?: string }) {
  const style: CSSProperties = width ? { width, flexShrink: 0 } : { flex: '1 1 0', minWidth: NAME_MIN };
  return (
    <div role="cell" className={cn('flex items-center overflow-hidden px-2.5', className)} style={style}>
      {children}
    </div>
  );
}

function HeaderCell({ width, children }: { width?: number; children: ReactNode }) {
  const style: CSSProperties = width ? { width, flexShrink: 0 } : { flex: '1 1 0', minWidth: NAME_MIN };
  return (
    <div
      role="columnheader"
      className="truncate px-2.5 text-[11px] font-semibold uppercase tracking-[0.06em] text-fg-2"
      style={style}
    >
      {children}
    </div>
  );
}

function TrackRow({ id, index, top }: { id: number; index: number; top: number }) {
  const row = useLibrary((s) => s.rows[id]);
  const selected = useLibrary((s) => s.selected === id);
  const select = useLibrary((s) => s.select);
  const target = useSettings((s) => s.target);
  const ceiling = useSettings((s) => s.ceiling);
  const bpmRange = useSettings((s) => s.bpmRange);
  if (!row) return null;
  return (
    <div
      role="row"
      id={rowDomId(id)}
      aria-selected={selected}
      aria-rowindex={index + 2}
      data-state={row.state}
      onClick={() => select(id)}
      className={cn(
        'absolute left-0 right-0 flex items-center border-b border-[#1c2129]',
        index % 2 === 1 && 'bg-[rgba(255,255,255,.015)]',
        selected && 'bg-bg-2 shadow-[inset_2px_0_0_var(--sc-accent)]',
        row.state === 'skipped' && 'text-fg-2',
      )}
      style={{ top, height: ROW_HEIGHT }}
    >
      <Cell>
        <NameCell row={row} />
      </Cell>
      <Cell width={COLUMNS.spec}>
        <SpecCell row={row} />
      </Cell>
      <Cell width={COLUMNS.loudness}>
        <LoudnessCell row={row} target={target} />
      </Cell>
      <Cell width={COLUMNS.tp}>
        <TpCell row={row} ceiling={ceiling} />
      </Cell>
      <Cell width={COLUMNS.bpm}>
        <BpmCell row={row} />
      </Cell>
      <Cell width={COLUMNS.action}>
        <ActionCell row={row} bpmRange={bpmRange} />
      </Cell>
      <Cell width={COLUMNS.status}>
        <StatusPill row={row} />
      </Cell>
    </div>
  );
}

/** The library table: a sticky header and only the rows in view (plus a margin). */
export function Table() {
  const ids = useLibrary(useShallow(visibleIds));
  const selected = useLibrary((s) => s.selected);
  const mode = useSettings((s) => s.mode);
  const ref = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setHeight(el.clientHeight);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Keep the selected row in view when the selection moves, and only then: rows being
  // classified during a job must not pull a user who scrolled away back to the selection.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || selected === null) return;
    const i = visibleIds(useLibrary.getState()).indexOf(selected);
    if (i < 0) return;
    const top = i * ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) {
      el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
    }
  }, [selected]);

  const inView = Math.max(Math.ceil(height / ROW_HEIGHT), MIN_ROWS);
  const firstVisible = Math.floor(scrollTop / ROW_HEIGHT);
  const first = Math.max(0, firstVisible - OVERSCAN);
  const last = Math.min(ids.length, firstVisible + inView + OVERSCAN);

  return (
    <div
      role="grid"
      aria-label="Tracks"
      aria-rowcount={ids.length + 1}
      aria-readonly="true"
      className="flex min-h-0 flex-1 flex-col overflow-x-auto"
    >
      <div className="flex min-h-0 min-w-[1046px] flex-1 flex-col">
        <div role="row" aria-rowindex={1} className="flex h-[34px] shrink-0 items-center border-b border-line bg-bg-1">
          <HeaderCell>Name</HeaderCell>
          <HeaderCell width={COLUMNS.spec}>Spec</HeaderCell>
          <HeaderCell width={COLUMNS.loudness}>
            {mode === 'dj' ? 'Loudness · S-P95' : 'Loudness · Integrated'}
          </HeaderCell>
          <HeaderCell width={COLUMNS.tp}>TP</HeaderCell>
          <HeaderCell width={COLUMNS.bpm}>BPM</HeaderCell>
          <HeaderCell width={COLUMNS.action}>Action</HeaderCell>
          <HeaderCell width={COLUMNS.status}>Status</HeaderCell>
        </div>
        <div
          ref={ref}
          role="rowgroup"
          tabIndex={0}
          aria-activedescendant={selected !== null && ids.includes(selected) ? rowDomId(selected) : undefined}
          className="relative min-h-0 flex-1 overflow-y-auto focus-visible:outline-offset-[-2px]"
          onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
          data-testid="table-body"
        >
          <div style={{ height: ids.length * ROW_HEIGHT }} />
          {ids.slice(first, last).map((id, k) => (
            <TrackRow key={id} id={id} index={first + k} top={(first + k) * ROW_HEIGHT} />
          ))}
          {ids.length === 0 && (
            <div className="absolute inset-x-0 top-10 text-center text-fg-2">No tracks match this filter.</div>
          )}
        </div>
      </div>
    </div>
  );
}
