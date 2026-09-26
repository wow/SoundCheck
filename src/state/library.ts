import { create } from 'zustand';
import type {
  FileEntry,
  IpcError,
  JobEvent,
  Plan,
  Replan,
  RowAnalysis,
  SessionSnapshot,
} from '@/lib/ipc';

/** Where a row stands, as the Status column shows it. */
export type RowState =
  | 'queued'
  | 'analysing'
  | 'analysed'
  | 'needsReview'
  | 'skipped'
  | 'error'
  | 'cancelled';

export interface Row {
  entry: FileEntry;
  state: RowState;
  /** Analysis progress while analysing, 0..1. */
  progress: number;
  analysis?: RowAnalysis;
  plan?: Plan;
  /** The settings revision `plan` was decided with; a newer plan always wins. */
  planRevision?: number;
  error?: IpcError;
  /** The BPM range the row was last analysed with, to know when it is stale. */
  analysedRange?: [number, number];
}

export type Filter = 'all' | 'review' | 'short' | 'skipped' | 'done';

export interface JobProgress {
  jobId: number | null;
  /** The rows this job was started with. */
  fileIds: number[];
  done: number;
  total: number;
  etaMs: number | null;
  realtimeX: number | null;
  startedAt: number;
}

export interface LastBatch {
  analysed: number;
  review: number;
  failed: number;
  cancelled: boolean;
  seconds: number;
}

interface LibraryState {
  rows: Record<number, Row>;
  order: number[];
  filter: Filter;
  query: string;
  selected: number | null;
  job: JobProgress | null;
  lastBatch: LastBatch | null;
  /** Why the last job could not start, until the next one does. */
  aborted: IpcError | null;
  /** A problem to show above the table (for example, settings the engine refused). */
  notice: string | null;
  add(entries: FileEntry[]): void;
  /** Marks rows queued for a job about to start with `range`. */
  queue(fileIds: number[], range: [number, number]): void;
  applyEvent(event: JobEvent): void;
  applyReplan(replan: Replan): void;
  /** Rebuilds the table from the engine's session after the window reloaded. */
  restore(snapshot: SessionSnapshot, range: [number, number]): void;
  setNotice(notice: string | null): void;
  /** Empties the list (the engine forgets it too, through `clearList`). */
  clear(): void;
  setFilter(filter: Filter): void;
  setQuery(query: string): void;
  select(fileId: number | null): void;
  moveSelection(delta: number): void;
  /** Selects the next visible Needs review row after the selection, wrapping; returns it. */
  selectNextReview(): number | null;
}

function stateOf(plan: Plan): RowState {
  switch (plan.status) {
    case 'needsReview':
      return 'needsReview';
    case 'skipped':
      return 'skipped';
    default:
      return 'analysed';
  }
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
      return false;
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
    if (isShort(row)) c.short += 1;
  }
  return c;
}

/** Rows the Analyse button would (re)run: failed, cancelled, queued, or analysed with another BPM range. */
export function staleIds(s: Pick<LibraryState, 'rows' | 'order'>, range: [number, number]): number[] {
  return s.order.filter((id) => {
    const row = s.rows[id];
    if (!row) return false;
    if (row.state === 'error' || row.state === 'cancelled' || row.state === 'queued') return true;
    const r = row.analysedRange;
    return r !== undefined && (r[0] !== range[0] || r[1] !== range[1]);
  });
}

/** Applies `patch` to each listed row in one copy of the map. */
function patchRows(
  rows: Record<number, Row>,
  patches: Iterable<[number, (row: Row) => Partial<Row> | null]>,
): Record<number, Row> {
  let next: Record<number, Row> | null = null;
  for (const [id, patch] of patches) {
    const row = (next ?? rows)[id];
    if (!row) continue;
    const change = patch(row);
    if (!change) continue;
    next ??= { ...rows };
    next[id] = { ...row, ...change };
  }
  return next ?? rows;
}

function one(rows: Record<number, Row>, id: number, patch: (row: Row) => Partial<Row> | null) {
  return patchRows(rows, [[id, patch]]);
}

export const useLibrary = create<LibraryState>()((set, get) => ({
  rows: {},
  order: [],
  filter: 'all',
  query: '',
  selected: null,
  job: null,
  lastBatch: null,
  aborted: null,
  notice: null,
  add(entries) {
    const { rows, order, selected } = get();
    const next = { ...rows };
    const ids: number[] = [];
    for (const entry of entries) {
      if (next[entry.fileId]) continue;
      next[entry.fileId] = { entry, state: 'queued', progress: 0 };
      ids.push(entry.fileId);
    }
    set({ rows: next, order: [...order, ...ids], selected: selected ?? ids[0] ?? null });
  },
  queue(fileIds, range) {
    const rows = patchRows(
      get().rows,
      fileIds.map((id) => [id, () => ({ state: 'queued', progress: 0, error: undefined, analysedRange: range })]),
    );
    set({
      rows,
      aborted: null,
      job: {
        jobId: null,
        fileIds,
        done: 0,
        total: fileIds.length,
        etaMs: null,
        realtimeX: null,
        startedAt: Date.now(),
      },
    });
  },
  applyEvent(event) {
    const { rows, job } = get();
    switch (event.type) {
      case 'started':
        set({
          rows: one(rows, event.fileId, () => ({ state: 'analysing', progress: 0 })),
          job: job && job.jobId === null ? { ...job, jobId: event.jobId } : job,
        });
        break;
      case 'progress':
        set({ rows: one(rows, event.fileId, () => ({ state: 'analysing', progress: event.fraction })) });
        break;
      case 'analysed':
        set({
          rows: one(rows, event.fileId, (row) => {
            // A replan newer than the one this event was decided with already set the plan.
            const newer = row.plan !== undefined && (row.planRevision ?? -1) > event.revision;
            const plan = newer && row.plan ? row.plan : event.plan;
            return {
              state: stateOf(plan),
              progress: 1,
              analysis: event.row,
              plan,
              planRevision: newer ? row.planRevision : event.revision,
            };
          }),
        });
        break;
      case 'failed':
        set({ rows: one(rows, event.fileId, () => ({ state: 'error', progress: 0, error: event.error })) });
        break;
      case 'cancelled':
        set({ rows: one(rows, event.fileId, () => ({ state: 'cancelled', progress: 0 })) });
        break;
      case 'batch':
        if (job) {
          set({
            job: {
              ...job,
              jobId: event.jobId,
              done: event.done,
              total: event.total,
              etaMs: event.etaMs,
              realtimeX: event.realtimeX,
            },
          });
        }
        break;
      case 'aborted':
        set({ aborted: event.error });
        break;
      case 'finished': {
        // A job that ends after the list was cleared leaves nothing to report.
        if (!job) break;
        const ids = job.fileIds;
        // Rows of this job still waiting (it was aborted or cancelled early) no longer are.
        const next = patchRows(
          rows,
          ids.map((id) => [
            id,
            (row) => (row.state === 'queued' || row.state === 'analysing' ? { state: 'cancelled', progress: 0 } : null),
          ]),
        );
        const mine = ids.map((id) => next[id]).filter((r): r is Row => r !== undefined);
        set({
          rows: next,
          job: null,
          lastBatch: {
            analysed: mine.filter((r) => r.analysis !== undefined && (r.state === 'analysed' || r.state === 'needsReview' || r.state === 'skipped')).length,
            review: mine.filter((r) => r.state === 'needsReview').length,
            failed: mine.filter((r) => r.state === 'error').length,
            cancelled: event.cancelled,
            seconds: (Date.now() - job.startedAt) / 1000,
          },
        });
        break;
      }
    }
  },
  applyReplan({ revision, plans }) {
    set({
      notice: null,
      rows: patchRows(
        get().rows,
        plans.map(({ fileId, plan }) => [
          fileId,
          (row) => {
            if ((row.planRevision ?? -1) > revision) return null;
            // Rows still in flight keep their status until their analysis arrives.
            return row.analysis
              ? { plan, planRevision: revision, state: stateOf(plan) }
              : { plan, planRevision: revision };
          },
        ]),
      ),
    });
  },
  restore(snapshot, range) {
    const rows: Record<number, Row> = {};
    const order: number[] = [];
    for (const r of snapshot.rows) {
      const id = r.entry.fileId;
      order.push(id);
      rows[id] =
        r.row && r.plan
          ? {
              entry: r.entry,
              state: stateOf(r.plan),
              progress: 1,
              analysis: r.row,
              plan: r.plan,
              planRevision: snapshot.revision,
              analysedRange: range,
            }
          : { entry: r.entry, state: 'cancelled', progress: 0 };
    }
    set({ rows, order, selected: order[0] ?? null, job: null });
  },
  setNotice(notice) {
    set({ notice });
  },
  clear() {
    set({ rows: {}, order: [], selected: null, job: null, lastBatch: null, aborted: null, filter: 'all', query: '' });
  },
  setFilter(filter) {
    set({ filter });
  },
  setQuery(query) {
    set({ query });
  },
  select(fileId) {
    set({ selected: fileId });
  },
  moveSelection(delta) {
    const ids = visibleIds(get());
    if (ids.length === 0) return;
    const at = get().selected === null ? -1 : ids.indexOf(get().selected as number);
    const next = Math.min(ids.length - 1, Math.max(0, at + delta));
    set({ selected: ids[next] ?? null });
  },
  selectNextReview() {
    const s = get();
    const ids = visibleIds(s);
    const review = ids.filter((id) => s.rows[id]?.state === 'needsReview');
    if (review.length === 0) return null;
    const start = s.selected === null ? -1 : s.order.indexOf(s.selected);
    const next = review.find((id) => s.order.indexOf(id) > start) ?? review[0] ?? null;
    set({ selected: next });
    return next;
  },
}));
