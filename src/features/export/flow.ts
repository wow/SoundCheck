import { create } from 'zustand';
import {
  errorCopy,
  ipcErrorOf,
  planExport,
  type ExportPreview,
  type ExportPreviewRow,
  type ExportRequest,
} from '@/lib/ipc';
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

/** Why Export cannot be pressed now, plainly; `null` when it can. */
export function exportBlock(g: ExportGate): string | null {
  const { work } = g;
  if (g.recoveryRunning) return 'Checking for changes a crash interrupted.';
  if (g.busy) return 'A job is running.';
  if (work.reverting) return 'A revert is running.';
  if (g.xmlWriting) return 'The rekordbox XML is being written.';
  if (!g.hasRows) return null;
  if (g.needsFolder) return 'Choose a folder';
  if (work.needAnalyse > 0) {
    const tracks = work.needAnalyse === 1 ? '1 track needs' : `${work.needAnalyse} tracks need`;
    return `${tracks} Analyse for this BPM range.`;
  }
  if (work.fresh.length === 0 && work.differing.length === 0) {
    if (!work.anyExported) return work.along.length > 0 ? 'Nothing new to export.' : 'Analyse the tracks first.';
    return work.anyRevertible
      ? 'Nothing new to export. Revert a track, or save a new grid, to export it again.'
      : 'Nothing new to export. Save a new grid, or change a setting, to export a track again.';
  }
  return null;
}

/** The rows a request goes out with: planned, and the differing ones only when asked. */
export function selectedRows(flow: Pick<ExportFlow, 'preview' | 'differing' | 'again'>): ExportPreviewRow[] {
  const rows = flow.preview?.rows ?? [];
  return flow.again ? rows : rows.filter((r) => !flow.differing.includes(r.fileId));
}

/** Bumped by every opening and closing, so an old answer lands nowhere. */
let attempt = 0;

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
  const fileIds = ids.filter(
    (id) => work.fresh.includes(id) || work.differing.includes(id) || work.along.includes(id),
  );
  if (fileIds.length === 0 || useRecovery.getState().running) {
    dismissExportConfirm();
    return;
  }
  const request: ExportRequest = {
    fileIds,
    settings: now.settings,
    outDir: now.outDir,
    confirmed: null,
  };
  const differingRows = work.differing.flatMap((id) => (lib.rows[id] ? [lib.rows[id]] : []));
  useExportFlow.setState({
    request,
    leftOut: rows.filter((r) => r.analysis === undefined).length,
    differing: work.differing,
    along: work.along,
    ...againCopy(differingRows, now),
  });
  try {
    const preview = await planExport(request);
    if (mine === attempt) useExportFlow.setState({ phase: 'ready', preview });
  } catch (err) {
    if (mine === attempt) useExportFlow.setState({ phase: 'failed', error: errorCopy(ipcErrorOf(err)) });
  }
}

/** Ticks or unticks exporting the differing rows again. */
export function setExportAgain(again: boolean): void {
  useExportFlow.setState({ again });
}

/** Starts the export the step shows: the rows it could plan, each as it planned them. */
export function confirmExport(): void {
  const flow = useExportFlow.getState();
  const { request, preview, phase } = flow;
  if (phase !== 'ready' || !request || !preview) return;
  const planned = selectedRows(flow).filter((r) => r.row && r.identity);
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
          files: planned.map((r) => ({ fileId: r.fileId, identity: r.identity as string })),
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
