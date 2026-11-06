import { create } from 'zustand';
import {
  exportEnded,
  exportPatch,
  exportQueued,
  isExporting,
  newJob,
  restingState,
  revertedPatch,
  rowFromSession,
  tallied,
  writtenWithoutAnalysis,
} from './exportRows';
import { visibleIds } from './librarySelectors';
import type { LibraryState, Row } from './libraryTypes';

export { writtenWithoutAnalysis };
export * from './librarySelectors';
export type * from './libraryTypes';

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
  lastExport: null,
  lastRun: null,
  artefacts: null,
  artefactsFailed: null,
  xmlLeftOut: 0,
  xmlWriting: false,
  locked: null,
  windingJobs: [],
  aborted: null,
  abortedKind: 'analysis',
  notice: null,
  info: null,
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
      fileIds.map((id) => [
        id,
        () => ({
          state: 'queued',
          progress: 0,
          error: undefined,
          exportError: undefined,
          analysedRange: range,
        }),
      ]),
    );
    set({
      rows,
      aborted: null,
      abortedKind: 'analysis',
      job: newJob('analysis', fileIds),
    });
  },
  beginExport(fileIds, planned) {
    const byId = new Map(planned.map((p) => [p.fileId, p]));
    set({
      rows: patchRows(
        get().rows,
        fileIds.map((id) => [id, (row) => exportQueued(row, byId.get(id))]),
      ),
      aborted: null,
      abortedKind: 'export',
      artefacts: null,
      artefactsFailed: null,
      xmlLeftOut: 0,
      lastExport: null,
      locked: new Set(fileIds),
      job: newJob('export', fileIds),
    });
  },
  setReverting(fileId) {
    set({
      rows: one(get().rows, fileId, () => ({
        reverting: true,
        revertError: undefined,
        revertNotes: undefined,
        exportError: undefined,
      })),
    });
  },
  applyRevert(fileId, result) {
    set({
      rows: one(get().rows, fileId, (row) => revertedPatch(row, result)),
    });
  },
  revertFailed(fileId, error) {
    set({ rows: one(get().rows, fileId, () => ({ reverting: false, revertError: error })) });
  },
  setXmlWriting(xmlWriting) {
    set({ xmlWriting });
  },
  applyXml({ artefacts, notListed }) {
    const why = new Map(notListed.map((n) => [n.fileId, n.why]));
    set({
      xmlWriting: false,
      artefacts,
      artefactsFailed: null,
      xmlLeftOut: notListed.length,
      rows: patchRows(
        get().rows,
        get().order.map((id) => [
          id,
          (row) => {
            const left = why.get(id);
            return left === row.xmlNotListed ? null : { xmlNotListed: left };
          },
        ]),
      ),
    });
  },
  setXmlFailed(error) {
    set({ xmlWriting: false, artefacts: null, artefactsFailed: error, xmlLeftOut: 0 });
  },
  windDownEnded() {
    const { lastExport, windingJobs } = get();
    // Only the job the reload stopped: an analysis it stopped says nothing about an older export.
    const stopped = lastExport?.jobId !== undefined && windingJobs.includes(lastExport.jobId);
    set({
      locked: null,
      windingJobs: [],
      lastExport: lastExport && stopped ? { ...lastExport, stoppedByReload: true } : lastExport,
    });
  },
  applyRowUpdate(update) {
    get().applyEvent({
      type: 'analysed',
      jobId: 0,
      fileId: update.fileId,
      row: update.row,
      plan: update.plan,
      revision: update.revision,
      // The engine says whether the saved grid differs from what the export wrote.
      exportStale: update.exportStale,
    });
  },
  applyEvent(event) {
    const { rows, job } = get();
    // A file event of a running export; `job` is null once the list was cleared. A row outside the
    // batch (added since, its grid saved meanwhile) goes on as any other.
    if (job?.kind === 'export' && 'fileId' in event && job.fileIds.includes(event.fileId)) {
      set({
        rows: one(rows, event.fileId, (row) => exportPatch(row, event)),
        job: { ...job, jobId: job.jobId ?? event.jobId, tally: tallied(job.tally, event) },
      });
      return;
    }
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
              state: restingState(plan, row.export),
              progress: 1,
              // The engine says whether what its export wrote still stands for the file.
              exportStale: event.exportStale,
              changedOnDisk: false,
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
      case 'artefacts':
        set({ artefacts: event.artefacts, artefactsFailed: null, xmlLeftOut: 0 });
        break;
      case 'artefactsFailed':
        set({ artefacts: null, artefactsFailed: event.error });
        break;
      case 'aborted':
        set({ aborted: event.error });
        break;
      case 'finished': {
        // A job that ends after the list was cleared leaves nothing to report.
        if (!job) break;
        const ids = job.fileIds;
        if (job.kind === 'export') {
          // Rows it never reached (it was refused or cancelled early) are as they were.
          const next = patchRows(
            rows,
            ids.map((id) => [id, (row) => (isExporting(row.state) ? exportEnded(row) : null)]),
          );
          set({
            rows: next,
            job: null,
            locked: null,
            lastRun: 'export',
            lastExport: {
              ...job.tally,
              cancelled: event.cancelled,
              seconds: (Date.now() - job.startedAt) / 1000,
              fileIds: ids,
            },
          });
          break;
        }
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
          lastRun: 'analysis',
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
            return row.analysis && !isExporting(row.state) && row.state !== 'failed'
              ? { plan, planRevision: revision, state: restingState(plan, row.export) }
              : { plan, planRevision: revision };
          },
        ]),
      ),
    });
  },
  restore(snapshot) {
    const { rows: before, selected } = get();
    const live = snapshot.runningJobs.length > 0;
    const rows: Record<number, Row> = {};
    const order: number[] = [];
    for (const r of snapshot.rows) {
      const id = r.entry.fileId;
      order.push(id);
      const row = rowFromSession(r, snapshot.revision, live);
      // Added while the job winds down, and waiting for it: still waiting.
      rows[id] = before[id]?.state === 'queued' && !row.analysis ? before[id] : row;
    }
    set({
      rows,
      order,
      selected: selected !== null && rows[selected] ? selected : (order[0] ?? null),
      job: null,
      locked: live ? 'all' : null,
      windingJobs: [...new Set([...get().windingJobs, ...snapshot.runningJobs])],
      artefacts: snapshot.artefacts,
      artefactsFailed: snapshot.artefactsFailed,
      lastExport: snapshot.lastExport,
      lastRun: snapshot.lastExport ? 'export' : null,
    });
  },
  markChanged(fileIds) {
    set({ rows: patchRows(get().rows, fileIds.map((id) => [id, () => ({ changedOnDisk: true })])) });
  },
  markRefused(refusals) {
    set({
      rows: patchRows(
        get().rows,
        refusals.map(({ fileId, error }) => [fileId, () => ({ state: 'failed', progress: 0, exportError: error })]),
      ),
    });
  },
  setNotice(notice) {
    set({ notice });
  },
  setInfo(info) {
    set({ info });
  },
  clear() {
    set({
      rows: {},
      order: [],
      selected: null,
      job: null,
      lastBatch: null,
      lastExport: null,
      lastRun: null,
      artefacts: null,
      artefactsFailed: null,
      xmlLeftOut: 0,
      locked: null,
      windingJobs: [],
      aborted: null,
      info: null,
      filter: 'all',
      query: '',
    });
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
