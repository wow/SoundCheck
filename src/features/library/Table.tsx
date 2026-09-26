import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { cn } from '@/lib/utils';
import { useLibrary, visibleIds } from '@/state/library';
import { useSettings } from '@/state/settings';
import { layoutFor, minWidth, rowDomId, type Layout } from './columns';
import { ActionCell, BpmCell, LoudnessCell, NameCell, SpecCell, StatusPill, TpCell } from './cells';

/** Row height in px; rows are fixed-height so only the visible ones are rendered. */
export const ROW_HEIGHT = 44;
const OVERSCAN = 8;
/** Rows rendered before the viewport has been measured. */
const MIN_ROWS = 20;

function Cell({
  width,
  nameMin,
  children,
  className,
}: {
  width?: number;
  nameMin?: number;
  children: ReactNode;
  className?: string;
}) {
  const style: CSSProperties = width
    ? { width, flexShrink: 0 }
    : { flex: '1 1 0', minWidth: nameMin };
  return (
    <div
      role="cell"
      className={cn('flex items-center overflow-hidden px-2.5', className)}
      style={style}
    >
      {children}
    </div>
  );
}

function HeaderCell({
  width,
  nameMin,
  children,
}: {
  width?: number;
  nameMin?: number;
  children: ReactNode;
}) {
  const style: CSSProperties = width
    ? { width, flexShrink: 0 }
    : { flex: '1 1 0', minWidth: nameMin };
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

function TrackRow({
  id,
  index,
  top,
  layout,
}: {
  id: number;
  index: number;
  top: number;
  layout: Layout;
}) {
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
      <Cell nameMin={layout.nameMin}>
        <NameCell row={row} flagUnsafe={layout.spec === null} />
      </Cell>
      {layout.spec !== null && (
        <Cell width={layout.spec}>
          <SpecCell row={row} />
        </Cell>
      )}
      <Cell width={layout.loudness}>
        <LoudnessCell row={row} target={target} bar={layout.bar} />
      </Cell>
      <Cell width={layout.tp}>
        <TpCell row={row} ceiling={ceiling} />
      </Cell>
      <Cell width={layout.bpm}>
        <BpmCell row={row} />
      </Cell>
      <Cell width={layout.action}>
        <ActionCell row={row} bpmRange={bpmRange} />
      </Cell>
      <Cell width={layout.status}>
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
  const gridRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);
  const [width, setWidth] = useState(0);
  const layout = layoutFor(width);

  useLayoutEffect(() => {
    const el = ref.current;
    const grid = gridRef.current;
    if (!el || !grid) return;
    // Height from the scrolling body; width from the grid, which follows the window even when
    // the rows inside have reached their minimum width.
    const measure = () => {
      setHeight(el.clientHeight);
      setWidth(grid.clientWidth);
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    ro.observe(grid);
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
      ref={gridRef}
      role="grid"
      aria-label="Tracks"
      aria-rowcount={ids.length + 1}
      aria-readonly="true"
      className="flex min-h-0 flex-1 flex-col overflow-x-auto"
    >
      <div className="flex min-h-0 flex-1 flex-col" style={{ minWidth: minWidth(layout) }}>
        <div
          role="row"
          aria-rowindex={1}
          className="flex h-[34px] shrink-0 items-center border-b border-line bg-bg-1"
        >
          <HeaderCell nameMin={layout.nameMin}>Name</HeaderCell>
          {layout.spec !== null && <HeaderCell width={layout.spec}>Spec</HeaderCell>}
          <HeaderCell width={layout.loudness}>
            {mode === 'dj'
              ? layout.compact
                ? 'S-P95'
                : 'Loudness · S-P95'
              : layout.compact
                ? 'Integrated'
                : 'Loudness · Integrated'}
          </HeaderCell>
          <HeaderCell width={layout.tp}>TP</HeaderCell>
          <HeaderCell width={layout.bpm}>BPM</HeaderCell>
          <HeaderCell width={layout.action}>Action</HeaderCell>
          <HeaderCell width={layout.status}>Status</HeaderCell>
        </div>
        <div
          ref={ref}
          role="rowgroup"
          tabIndex={0}
          aria-activedescendant={
            selected !== null && ids.includes(selected) ? rowDomId(selected) : undefined
          }
          className="relative min-h-0 flex-1 overflow-y-auto focus-visible:outline-offset-[-2px]"
          onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
          data-testid="table-body"
        >
          <div style={{ height: ids.length * ROW_HEIGHT }} />
          {ids.slice(first, last).map((id, k) => (
            <TrackRow
              key={id}
              id={id}
              index={first + k}
              top={(first + k) * ROW_HEIGHT}
              layout={layout}
            />
          ))}
          {ids.length === 0 && (
            <div className="absolute inset-x-0 top-10 text-center text-fg-2">
              No tracks match this filter.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
