import { create } from 'zustand';
import { errorCopy, ipcErrorOf, planExport, type ExportPreview, type ExportRequest } from '@/lib/ipc';
import { pipeline } from '@/features/pipeline/actions';
import { exportableIds, useLibrary, type Row } from '@/state/library';
import { usePanels } from '@/state/panels';
import { useRecovery } from '@/state/recovery';
import { exportSettings, useSettings, type SavedSettings } from '@/state/settings';
import { closeGridViewOf } from './actions';
import { batchChoice } from './mode';

/**
 * The confirm step: Export asks the engine what it would do with each file, shows the numbers, and
 * only then starts the job. Nothing is written before the user says so.
 */
interface ExportFlow {
  phase: 'closed' | 'planning' | 'ready' | 'failed';
  request: ExportRequest | null;
  preview: ExportPreview | null;
  /** Why it cannot be planned, in the words the user sees. */
  error: string | null;
  /** Tracks in the list that are not part of it, for not being analysed. */
  leftOut: number;
}

const CLOSED: ExportFlow = { phase: 'closed', request: null, preview: null, error: null, leftOut: 0 };

export const useExportFlow = create<ExportFlow>()(() => CLOSED);

export interface ExportGate {
  exportable: number;
  hasRows: boolean;
  /** Some row's file is exported, and only a revert or a saved grid would export it again. */
  anyExported: boolean;
  busy: boolean;
  reverting: boolean;
  xmlWriting: boolean;
  recoveryRunning: boolean;
  needsFolder: boolean;
}

/** Why Export cannot be pressed now, plainly; `null` when it can. */
export function exportBlock(g: ExportGate): string | null {
  if (g.recoveryRunning) return 'Checking for changes a crash interrupted.';
  if (g.busy) return 'A job is running.';
  if (g.reverting) return 'A revert is running.';
  if (g.xmlWriting) return 'The rekordbox XML is being written.';
  if (!g.hasRows) return null;
  if (g.exportable === 0) {
    return g.anyExported
      ? 'Nothing new to export. Revert a track, or save its grid, to export it again.'
      : 'Analyse the tracks first.';
  }
  if (g.needsFolder) return 'Choose a folder first.';
  return null;
}

/** The request for the rows as they stand and the settings as they are. */
export function exportRequest(s: SavedSettings, fileIds: number[], rows: Row[]): ExportRequest {
  return {
    fileIds,
    settings: exportSettings(s, batchChoice(s, rows).mode),
    outDir: s.place === 'folder' ? s.outDir : null,
  };
}

/** Bumped by every opening and closing, so an old answer lands nowhere. */
let attempt = 0;

/** Opens the confirm step and plans the export. */
export async function openExportConfirm(): Promise<void> {
  const lib = useLibrary.getState();
  const fileIds = exportableIds(lib);
  const settings = useSettings.getState();
  const all = lib.order.flatMap((id) => {
    const row = lib.rows[id];
    return row ? [row] : [];
  });
  if (fileIds.length === 0 || useRecovery.getState().running) return;
  // The drawer of a narrow window gives way: one modal panel at a time.
  usePanels.getState().setSettings(false);
  const request = exportRequest(settings, fileIds, all);
  const mine = ++attempt;
  useExportFlow.setState({
    ...CLOSED,
    phase: 'planning',
    request,
    leftOut: all.filter((r) => r.analysis === undefined).length,
  });
  try {
    const preview = await planExport(request);
    if (mine === attempt) useExportFlow.setState({ phase: 'ready', preview });
  } catch (err) {
    if (mine === attempt) useExportFlow.setState({ phase: 'failed', error: errorCopy(ipcErrorOf(err)) });
  }
}

/** Starts the export the step shows. */
export function confirmExport(): void {
  const { request, preview, phase } = useExportFlow.getState();
  if (phase !== 'ready' || !request || !preview) return;
  if (request.settings.place === 'inPlace') closeGridViewOf(request.fileIds, 'being exported in place');
  pipeline().send({ type: 'EXPORT', job: { request, planned: preview.rows } });
  dismissExportConfirm();
}

export function dismissExportConfirm(): void {
  attempt += 1;
  useExportFlow.setState(CLOSED);
}
