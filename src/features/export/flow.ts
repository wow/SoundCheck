import { create } from 'zustand';
import { errorCopy, ipcErrorOf, planExport, type ExportPreview, type ExportRequest } from '@/lib/ipc';
import { commitFocusedField } from '@/lib/focus';
import { pipeline } from '@/features/pipeline/actions';
import { settingsLanded } from '@/features/settings/sync';
import { useLibrary, type Row } from '@/state/library';
import { usePanels } from '@/state/panels';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { closeGridViewOf } from './actions';
import { batchChoice } from './mode';
import { againCopy, exportNow, exportWork, type ExportWork } from './work';

/**
 * The confirm step: Export asks the engine what it would do with each file, shows the numbers, and
 * only then starts the job. Nothing is written before the user says so.
 */
interface ExportFlow {
  phase: 'closed' | 'planning' | 'ready' | 'failed';
  /** The request, for every row the step may export. */
  request: ExportRequest | null;
  preview: ExportPreview | null;
  /** Why it cannot be planned, in the words the user sees. */
  error: string | null;
  /** Tracks in the list that are not part of it, for not being analysed. */
  leftOut: number;
  /** Exported with other settings: part of it only when asked. */
  differing: number[];
  /** Going along without being new work. */
  along: number[];
  /** Everything the step may plan, in table order, the differing rows among them. */
  planned: number[];
  /** Export the differing rows again too. */
  again: boolean;
  /** What the step says about those rows. */
  stays: string;
  offer: string;
}

const CLOSED: ExportFlow = {
  phase: 'closed',
  request: null,
  preview: null,
  error: null,
  leftOut: 0,
  differing: [],
  along: [],
  planned: [],
  again: false,
  stays: '',
  offer: '',
};

export const useExportFlow = create<ExportFlow>()(() => CLOSED);

export interface ExportGate {
  work: ExportWork;
  hasRows: boolean;
  busy: boolean;
  xmlWriting: boolean;
  recoveryRunning: boolean;
  needsFolder: boolean;
}

/** The reason when a folder is all that stands in the way: choosing one is the next step. */
export const CHOOSE_FOLDER = 'Choose a folder';

/** Why Export cannot be pressed now, plainly; `null` when it can. */
export function exportBlock(g: ExportGate): string | null {
  const { work } = g;
  if (g.recoveryRunning) return 'Checking for changes a crash interrupted.';
  if (g.busy) return 'A job is running.';
  if (work.reverting) return 'A revert is running.';
  if (g.xmlWriting) return 'The rekordbox XML is being written.';
  if (!g.hasRows) return null;
  if (work.needAnalyse > 0) {
    const tracks = work.needAnalyse === 1 ? '1 track needs' : `${work.needAnalyse} tracks need`;
    return `${tracks} Analyse for this BPM range.`;
  }
  if (!hasExportWork(work)) {
    if (!work.anyExported) return work.along.length > 0 ? 'Nothing new to export.' : 'Analyse the tracks first.';
    return work.anyRevertible
      ? 'Nothing new to export. Revert a track, or save a new grid, to export it again.'
      : 'Nothing new to export. Save a new grid, or change a setting, to export a track again.';
  }
  if (g.needsFolder) return CHOOSE_FOLDER;
  return null;
}

/** Whether there is anything for the confirm step to plan and possibly write. */
export function hasExportWork(work: ExportWork): boolean {
  return work.fresh.length > 0 || work.differing.length > 0 || work.retry.length > 0;
}

/** Bumped by every opening and closing, so an old answer lands nowhere. */
let attempt = 0;

/** The plan the engine is working on: it holds its files until it answers, whoever still wants it. */
let running: Promise<unknown> = Promise.resolve();

/** Opens the confirm step and plans the export, once what is being typed has reached the engine. */
export async function openExportConfirm(): Promise<void> {
  const mine = ++attempt;
  commitFocusedField();
  // The drawer of a narrow window gives way: one modal panel at a time.
  usePanels.getState().setSettings(false);
  useExportFlow.setState({ ...CLOSED, phase: 'planning' });
  await settingsLanded();
  if (mine !== attempt) return;
  const lib = useLibrary.getState();
  const settings = useSettings.getState();
  const ids = lib.order.filter((id) => lib.rows[id]);
  const rows = ids.map((id) => lib.rows[id] as Row);
  const now = exportNow(settings, batchChoice(settings, rows).mode);
  const work = exportWork(rows, ids, now);
  const planned = ids.filter(
    (id) =>
      work.fresh.includes(id) ||
      work.differing.includes(id) ||
      work.retry.includes(id) ||
      work.along.includes(id),
  );
  if (planned.length === 0 || useRecovery.getState().running) {
    dismissExportConfirm();
    return;
  }
  const differingRows = work.differing.flatMap((id) => (lib.rows[id] ? [lib.rows[id]] : []));
  useExportFlow.setState({
    request: { fileIds: [], settings: now.settings, outDir: now.outDir, confirmed: null },
    leftOut: rows.filter((r) => r.analysis === undefined).length,
    differing: work.differing,
    along: work.along,
    planned,
    ...againCopy(differingRows, now),
  });
  await planRows(mine);
}

/**
 * Asks the engine about exactly what would go out, so what the step says is what happens. One plan
 * at a time: a superseded plan still holds its files, so the next waits for it, and the step stays
 * in `planning` (Write disabled) until then. A plan superseded while waiting never starts.
 */
async function planRows(mine: number): Promise<void> {
  const { request, planned, differing, again } = useExportFlow.getState();
  if (!request) return;
  const fileIds = again ? planned : planned.filter((id) => !differing.includes(id));
  const asked = { ...request, fileIds };
  useExportFlow.setState({ phase: 'planning', preview: null, request: asked, error: null });
  await running;
  if (mine !== attempt) return;
  try {
    const answer = fileIds.length > 0 ? planExport(asked) : null;
    running = answer?.catch(() => undefined) ?? running;
    const preview = await answer;
    if (mine !== attempt) return;
    useExportFlow.setState({ phase: 'ready', preview });
    const changed = (preview?.rows ?? [])
      .filter((r) => r.error?.kind === 'fileChanged')
      .map((r) => r.fileId);
    if (changed.length > 0) useLibrary.getState().markChanged(changed);
  } catch (err) {
    if (mine === attempt) useExportFlow.setState({ phase: 'failed', error: errorCopy(ipcErrorOf(err)) });
  }
}

/** Ticks or unticks exporting the differing rows again, and plans what that leaves. */
export function setExportAgain(again: boolean): void {
  useExportFlow.setState({ again });
  void planRows(++attempt);
}

/** Starts the export the step shows: the rows it could plan, each as it planned them. */
export function confirmExport(): void {
  const { request, preview, phase } = useExportFlow.getState();
  if (phase !== 'ready' || !request || !preview) return;
  const planned = preview.rows.filter((r) => r.row && r.identity);
  const fileIds = planned.map((r) => r.fileId);
  if (request.settings.place === 'inPlace') closeGridViewOf(fileIds, 'being exported in place');
  pipeline().send({
    type: 'EXPORT',
    job: {
      request: {
        ...request,
        fileIds,
        confirmed: {
          revision: preview.revision,
          files: planned.map((r) => ({
            fileId: r.fileId,
            identity: r.identity as string,
            sameName: r.row?.action.type === 'skip' && r.row.action.reason.type === 'sameNameInBatch',
          })),
        },
      },
      planned,
    },
  });
  dismissExportConfirm();
}

export function dismissExportConfirm(): void {
  attempt += 1;
  useExportFlow.setState(CLOSED);
}
