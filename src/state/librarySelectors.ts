import type { Filter, LibraryState, Locked, Row } from './libraryTypes';

/** Whether a row must not be opened or reverted now, because a running batch holds its file. */
export function isLocked(locked: Locked, id: number): boolean {
  return locked === 'all' || (locked !== null && locked.has(id));
}

/** Whether a row is short of the target (the Short chip). */
export function isShort(row: Row): boolean {
  const g = row.plan?.gain;
  return row.analysis !== undefined && g?.type === 'gain' && g.shortByLu > 0;
}

function matchesFilter(row: Row, filter: Filter): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'review':
      return row.state === 'needsReview';
    case 'short':
      return isShort(row);
    case 'skipped':
      return row.state === 'skipped';
    case 'done':
      return row.state === 'done';
  }
}

function matchesQuery(row: Row, query: string): boolean {
  if (!query) return true;
  const q = query.toLocaleLowerCase();
  const { info, path } = row.entry;
  return [info.title, info.artist, info.album, path.split('/').pop()].some((s) =>
    s?.toLocaleLowerCase().includes(q),
  );
}

/** Row ids the table shows, in order. */
export function visibleIds(s: Pick<LibraryState, 'rows' | 'order' | 'filter' | 'query'>): number[] {
  return s.order.filter((id) => {
    const row = s.rows[id];
    return row !== undefined && matchesFilter(row, s.filter) && matchesQuery(row, s.query);
  });
}

/** The counts the filter chips show. */
export function counts(s: Pick<LibraryState, 'rows' | 'order'>): Record<Filter, number> {
  const c: Record<Filter, number> = { all: 0, review: 0, short: 0, skipped: 0, done: 0 };
  for (const id of s.order) {
    const row = s.rows[id];
    if (!row) continue;
    c.all += 1;
    if (row.state === 'needsReview') c.review += 1;
    if (row.state === 'skipped') c.skipped += 1;
    if (row.state === 'done') c.done += 1;
    if (isShort(row)) c.short += 1;
  }
  return c;
}

/** Rows the Analyse button would (re)run: failed, cancelled, queued, or analysed with another BPM range. */
export function staleIds(s: Pick<LibraryState, 'rows' | 'order'>, range: [number, number]): number[] {
  return s.order.filter((id) => {
    const row = s.rows[id];
    if (!row || row.reverting) return false;
    if (row.state === 'error' || row.state === 'cancelled' || row.state === 'queued') return true;
    const r = row.analysedRange;
    return r !== undefined && (r[0] !== range[0] || r[1] !== range[1]);
  });
}

/** Rows the rekordbox XML button lists: every analysed row, an exported one where its export is. */
export function xmlRowIds(s: Pick<LibraryState, 'rows' | 'order'>): number[] {
  return s.order.filter((id) => s.rows[id]?.analysis !== undefined);
}
