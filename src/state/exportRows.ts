import type {
  ExportPreviewRow,
  ExportRow,
  JobEvent,
  Plan,
  RevertResult,
  SessionRow,
} from '@/lib/ipc';
import type { JobKind, JobProgress, LastExport, Row, RowState } from './libraryTypes';

/**
 * What the export events do to a row. A row's `export` is what its Action column shows: the
 * confirm step's plan while it waits, then each step of the job. A failed or cancelled export
 * leaves the one before it standing, as the engine's session does.
 */

const EXPORT_FLOW: ReadonlySet<RowState> = new Set<RowState>([
  'exportQueued',
  'processing',
  'writing',
  'checking',
]);

/** A row an export is working on or about to. */
export function isExporting(state: RowState): boolean {
  return EXPORT_FLOW.has(state);
}

function stateOfPlan(plan: Plan | undefined, fallback: RowState): RowState {
  switch (plan?.status) {
    case 'needsReview':
      return 'needsReview';
    case 'skipped':
      return 'skipped';
    case undefined:
      return fallback;
    default:
      return 'analysed';
  }
}

/**
 * Where a row with no job on it stands: its export, unless its grid still asks for a look, which
 * an export does not settle.
 */
export function restingState(plan: Plan | undefined, standing: ExportRow | undefined): RowState {
  if (standing?.stage === 'skipped') return 'skipped';
  if (standing?.stage === 'done' && plan?.status !== 'needsReview') return 'done';
  return stateOfPlan(plan, 'cancelled');
}

/** The rows of a new export: waiting, showing what the confirm step said will happen. */
export function exportQueued(row: Row, planned: ExportPreviewRow | undefined): Partial<Row> {
  return {
    state: 'exportQueued',
    progress: 0,
    exportBefore: row.export,
    export: planned?.row ?? row.export,
    exportError: undefined,
    revertError: undefined,
    revertNotes: undefined,
    xmlNotListed: undefined,
  };
}

/** A row whose export ended without changing the file. */
export function exportEnded(row: Row): Partial<Row> {
  return {
    state: restingState(row.plan, row.exportBefore),
    progress: 0,
    export: row.exportBefore,
    exportBefore: undefined,
  };
}

/** What `event` does to a row of a running export; `null` for events that are not about it. */
export function exportPatch(row: Row, event: JobEvent): Partial<Row> | null {
  switch (event.type) {
    case 'started':
      return { state: 'processing', progress: 0 };
    case 'progress':
      return { state: 'processing', progress: event.fraction };
    // The plan is known: the write starts now.
    case 'processing':
      return { state: 'writing', export: event.row, revertible: event.row.revertible };
    // The file is in place and verified; only the grid check runs from here on.
    case 'written':
      return { state: 'checking', export: event.row, revertible: event.row.revertible };
    case 'exportSkipped':
      return {
        state: 'skipped',
        progress: 0,
        export: event.row,
        exportBefore: undefined,
        revertible: event.row.revertible,
      };
    case 'exportDone':
      return { ...doneOnDisk(row, event), export: event.row, exportBefore: undefined, exportStale: false };
    case 'failed':
      return { ...exportEnded(row), state: 'failed', exportError: event.error };
    case 'cancelled':
      return exportEnded(row);
    default:
      return null;
  }
}

/**
 * A finished file, by what became of the file. A copy in a folder: the source did not change and
 * the row stays. In place with an update: the row now is the written file. In place without one:
 * its analysis did not finish and what the row shows describes audio that is gone, so it is cleared
 * and analysed again.
 */
function doneOnDisk(row: Row, event: Extract<JobEvent, { type: 'exportDone' }>): Partial<Row> {
  const common = { progress: 1, revertible: event.row.revertible };
  if (!event.inPlace) return { ...common, state: restingState(row.plan, event.row) };
  const written = event.row.verification;
  const entry = written
    ? {
        ...row.entry,
        info: { ...row.entry.info, bitsPerSample: written.bitsOut, float: false, djUnsafe: null },
      }
    : row.entry;
  if (!event.update) {
    return {
      ...common,
      entry,
      state: 'queued',
      progress: 0,
      analysis: undefined,
      plan: undefined,
      planRevision: undefined,
      analysedRange: undefined,
    };
  }
  return {
    ...common,
    entry,
    state: restingState(event.update.plan, event.row),
    analysis: event.update.row,
    plan: event.update.plan,
    planRevision: event.update.revision,
  };
}

/** A row as the session holds it, after the window reloaded; `live`: a job is still running. */
export function rowFromSession(r: SessionRow, revision: number, range: [number, number], live: boolean): Row {
  const analysed = r.row && r.plan ? r : null;
  const standing = r.export ?? undefined;
  const row: Row = {
    entry: r.entry,
    state: 'cancelled',
    progress: analysed ? 1 : 0,
    revertible: r.revertible,
    exportStale: r.exportStale,
    ...(standing ? { export: standing } : {}),
    ...(analysed
      ? { analysis: r.row ?? undefined, plan: r.plan ?? undefined, planRevision: revision, analysedRange: range }
      : {}),
  };
  const inFlight = standing?.stage === 'processing' || standing?.stage === 'writing';
  if (inFlight && live) return { ...row, state: standing.stage === 'writing' ? 'checking' : 'writing' };
  if (inFlight) return { ...row, state: restingState(row.plan, undefined), export: undefined };
  if (analysed) return { ...row, state: restingState(row.plan, standing) };
  // Written in place and its analysis gone with what it described: it waits for a new one.
  return standing?.stage === 'done' ? { ...row, state: 'queued' } : row;
}

/** Rows written in place whose own analysis did not finish: they wait for a new one. */
export function writtenWithoutAnalysis(rows: Record<number, Row>, order: number[]): number[] {
  return order.filter((id) => {
    const row = rows[id];
    return row?.state === 'queued' && row.export?.stage === 'done' && !row.analysis;
  });
}

/** A row whose original is back. Without an update it could not be analysed: what it shows is gone. */
export function revertedPatch(row: Row, { entry, update, earlierChanges, notes }: RevertResult): Partial<Row> {
  const undone: Partial<Row> = {
    entry,
    reverting: false,
    revertible: earlierChanges > 0,
    revertNotes: notes,
    export: undefined,
    exportBefore: undefined,
    exportError: undefined,
    exportStale: false,
  };
  if (!update) {
    return {
      ...undone,
      state: 'queued',
      progress: 0,
      analysis: undefined,
      plan: undefined,
      planRevision: undefined,
      analysedRange: undefined,
    };
  }
  // A newer replan already decided this row's plan.
  const newer = row.plan !== undefined && (row.planRevision ?? -1) > update.revision;
  const plan = newer && row.plan ? row.plan : update.plan;
  return {
    ...undone,
    state: restingState(plan, undefined),
    progress: 1,
    analysis: update.row,
    plan,
    planRevision: newer ? row.planRevision : update.revision,
  };
}

export function newJob(kind: JobKind, fileIds: number[]): JobProgress {
  return {
    kind,
    jobId: null,
    fileIds,
    done: 0,
    total: fileIds.length,
    etaMs: null,
    realtimeX: null,
    startedAt: Date.now(),
  };
}

/** What became of the rows of an export. */
export function summarise(rows: Row[], cancelled: boolean, seconds: number): LastExport {
  const stage = (r: Row) => r.export?.stage;
  return {
    written: rows.filter((r) => stage(r) === 'done').length,
    xmlOnly: rows.filter((r) => r.export?.action.type === 'xmlOnly').length,
    skipped: rows.filter((r) => stage(r) === 'skipped' && r.export?.action.type !== 'xmlOnly').length,
    failed: rows.filter((r) => r.state === 'failed').length,
    cancelled,
    seconds,
  };
}
