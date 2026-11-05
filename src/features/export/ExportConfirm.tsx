import { Loader2 } from 'lucide-react';
import { useSelector } from '@xstate/react';
import { Modal } from '@/components/Modal';
import type { ExportPreview, ExportRequest } from '@/lib/ipc';
import { displayTitle } from '@/lib/format';
import { pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { confirmExport, dismissExportConfirm, useExportFlow } from './flow';
import { countPreview, shortPath, summaryLine, type ExportCounts } from './summary';

const buttonClass =
  'inline-flex h-[30px] items-center whitespace-nowrap rounded-[7px] border px-3 text-[13px] font-medium disabled:cursor-default disabled:opacity-45';

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

function settingsLine(req: ExportRequest): string {
  const s = req.settings;
  return [
    s.batchMode === 'prepare' ? 'Prepare' : 'Library',
    s.place === 'inPlace' ? 'in place with backup' : 'to folder',
    s.gridOnly ? 'grid only' : s.depth === null ? 'source depth' : `${s.depth}-bit`,
    s.tbpm ? 'TBPM' : null,
    s.xml ? 'rekordbox XML' : null,
  ]
    .filter(Boolean)
    .join(' · ');
}

/** Say it before anything is written: a cut moves every position kept outside the file. */
function CutWarning({ counts }: { counts: ExportCounts }) {
  if (counts.cut === 0) return null;
  return (
    <p className="rounded-md border border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.10)] px-3 py-2 text-[12.5px] leading-[1.45] text-warn">
      Positions shift on {plural(counts.cut, 'cut file')}, by up to{' '}
      <span className="font-mono">{counts.longestCutS.toFixed(2)} s</span>: cue points, loops and
      grids already saved for {counts.cut === 1 ? 'it' : 'them'} in a DJ app will sit late by the
      cut. For tracks already cued, use Library, which never changes the length.
    </p>
  );
}

/** Each file the step has something to say about: why it is skipped, or what to know about it. */
function Particulars({ preview }: { preview: ExportPreview }) {
  const rows = useLibrary((l) => l.rows);
  const items = preview.rows.flatMap(({ fileId, row, error }) => {
    const entry = rows[fileId]?.entry;
    const name = entry ? displayTitle(entry.info, entry.path) : `Track ${fileId}`;
    if (error) return [{ fileId, name, head: 'Cannot be planned', text: error.message }];
    if (!row) return [];
    const text = row.notes.join(' ');
    if (row.action.type === 'skip') return [{ fileId, name, head: 'Skipped', text }];
    if (row.action.type === 'xmlOnly') return [{ fileId, name, head: 'XML only', text }];
    return text ? [{ fileId, name, head: 'Note', text }] : [];
  });
  if (items.length === 0) return null;
  return (
    <ul aria-label="File by file" className="flex flex-col gap-2 border-t border-line pt-3">
      {items.map((item) => (
        <li key={item.fileId} className="text-[12px] leading-[1.4]">
          <div className="truncate font-medium text-fg-0" title={item.name}>
            {item.name}
          </div>
          <div className="text-fg-2">
            {item.head}: {item.text}
          </div>
        </li>
      ))}
    </ul>
  );
}

/** The summary shown before anything is written: the numbers are the engine's plan, file by file. */
export function ExportConfirm() {
  const { phase, request, preview, error, leftOut } = useExportFlow();
  const idle = useSelector(pipeline(), (p) => p.value === 'idle');
  if (phase === 'closed' || !request) return null;
  const counts = preview ? countPreview(preview) : null;
  const toDo = counts?.toDo ?? 0;
  return (
    <Modal label="Confirm export" onClose={dismissExportConfirm}>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-5 pb-4 pt-[18px]">
        <h2 className="text-[15px] font-semibold text-fg-0">
          Export {plural(request.fileIds.length, 'track')}
        </h2>
        {phase === 'planning' && (
          <p role="status" className="flex items-center gap-2 text-[13px] text-fg-1">
            <Loader2 className="size-3.5 motion-safe:animate-spin" aria-hidden="true" />
            Reading what each file needs…
          </p>
        )}
        {phase === 'failed' && (
          <p role="alert" className="text-[13px] leading-[1.45] text-err">
            {error}
          </p>
        )}
        {preview && counts && (
          <>
            <p role="status" className="font-mono text-[13px] leading-[1.45] text-fg-0">
              {summaryLine(preview, request.outDir)}
            </p>
            <p className="text-[12px] text-fg-2">{settingsLine(request)}</p>
            <CutWarning counts={counts} />
            {preview.artefactsFolder && (
              <p className="text-[12px] leading-[1.45] text-fg-2">
                The rekordbox XML and the grid report go to{' '}
                <span className="break-all font-mono text-fg-1">
                  {shortPath(preview.artefactsFolder)}
                </span>
                {preview.backupFolder ? ', in a folder named after this batch.' : '.'}
              </p>
            )}
            {leftOut > 0 && (
              <p className="text-[12px] text-fg-2">
                {plural(leftOut, 'track')} not analysed {leftOut === 1 ? 'is' : 'are'} left out.
              </p>
            )}
            <Particulars preview={preview} />
          </>
        )}
      </div>
      <div className="flex shrink-0 items-center justify-end gap-2 border-t border-line px-5 py-3">
        {!idle && phase === 'ready' && (
          <span className="mr-auto text-[12px] text-fg-2">Waiting for the running job.</span>
        )}
        <button
          type="button"
          onClick={dismissExportConfirm}
          className={`${buttonClass} border-line text-fg-0 hover:bg-bg-2`}
        >
          {phase === 'failed' ? 'Close' : 'Cancel'}
        </button>
        {phase !== 'failed' && (
          <button
            type="button"
            disabled={phase !== 'ready' || toDo === 0 || !idle}
            onClick={confirmExport}
            className={`${buttonClass} border-transparent bg-accent text-bg-0`}
          >
            {phase === 'ready' && toDo === 0 ? 'Nothing to export' : `Export ${toDo > 0 ? plural(toDo, 'file') : ''}`.trim()}
          </button>
        )}
      </div>
    </Modal>
  );
}
