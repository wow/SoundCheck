import { Loader2 } from 'lucide-react';
import { useSelector } from '@xstate/react';
import { Modal } from '@/components/Modal';
import { errorCopy, rowErrorCopy, sentence, type DecideSettings, type ExportPreviewRow, type ExportRequest } from '@/lib/ipc';
import { displayTitle, level } from '@/lib/format';
import { pipeline } from '@/features/pipeline/actions';
import { Toggle } from '@/features/settings/controls';
import { useLibrary } from '@/state/library';
import { confirmExport, dismissExportConfirm, setExportAgain, useExportFlow } from './flow';
import { flacConfirmText, flacWrites } from './notices';
import { countPreview, isRefusal, shortPath, summaryLine, type ExportCounts } from './summary';

const buttonClass =
  'inline-flex h-[30px] items-center whitespace-nowrap rounded-[7px] border px-3 text-[13px] font-medium disabled:cursor-default disabled:opacity-45';

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

function settingsLine(req: ExportRequest, decide: DecideSettings): string {
  const s = req.settings;
  return [
    `${decide.mode === 'dj' ? 'DJ' : 'Streaming'} target ${level(decide.target)} LUFS`,
    `ceiling ${level(decide.ceiling)} dBTP`,
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

/** Said once for the batch: rekordbox analyses a written FLAC again, before a CDJ plays it well. */
function FlacNotice({ rows: planned }: { rows: ExportPreviewRow[] }) {
  const count = useLibrary((l) => flacWrites(planned, l.rows));
  const text = flacConfirmText(count);
  if (!text) return null;
  return <p className="text-[12px] leading-[1.45] text-fg-1">{text}</p>;
}

/** Each file the step has something to say about: why it is skipped, or what to know about it. */
function Particulars({ rows: planned }: { rows: ExportPreviewRow[] }) {
  const rows = useLibrary((l) => l.rows);
  const items = planned.flatMap(({ fileId, row, error }) => {
    const entry = rows[fileId]?.entry;
    const name = entry ? displayTitle(entry.info, entry.path) : `Track ${fileId}`;
    if (error) {
      const text = entry ? rowErrorCopy(error, entry.path, 'confirm') : errorCopy(error, 'confirm');
      return [{ fileId, name, head: isRefusal(error) ? 'Refused' : 'Cannot be planned', text }];
    }
    if (!row) return [];
    const text = sentence(row.notes.join(' '));
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

/** What the step asks to do, the same words for its title and its button. */
function askText(counts: ExportCounts | null, planned: boolean): string {
  if (!counts) return planned ? 'Nothing to write' : 'Export';
  if (counts.written > 0) return `Write ${plural(counts.written, 'file')}`;
  return counts.xmlOnlyNew > 0 ? 'Update the rekordbox XML' : 'Nothing to write';
}

/** The summary shown before anything is written: the numbers are the engine's plan, file by file. */
export function ExportConfirm() {
  const { phase, request, preview, error, leftOut, differing, along, again, stays, offer } = useExportFlow();
  const idle = useSelector(pipeline(), (p) => p.value === 'idle');
  if (phase === 'closed') return null;
  const rows = preview?.rows ?? [];
  const counts = preview ? countPreview(rows, along) : null;
  const ask = askText(counts, phase === 'ready');
  const canWrite = counts !== null && (counts.written > 0 || counts.xmlOnlyNew > 0);
  const inFolder = preview && !preview.backupFolder;
  return (
    <Modal label="Confirm export" onClose={dismissExportConfirm}>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-5 pb-4 pt-[18px]">
        <h2 className="text-[15px] font-semibold text-fg-0">{ask}</h2>
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
        {preview && counts && request && (
          <>
            <p role="status" className="font-mono text-[13px] leading-[1.45] text-fg-0">
              {summaryLine(rows, preview, request.outDir)}
            </p>
            <p className="text-[12px] text-fg-2">{settingsLine(request, preview.decide)}</p>
            <CutWarning counts={counts} />
            <FlacNotice rows={rows} />
            {counts.replacing > 0 && (
              <p className="text-[12px] text-fg-1">
                {counts.replacing === 1
                  ? '1 file replaces your earlier copy.'
                  : `${counts.replacing} files replace your earlier copies.`}
              </p>
            )}
            {counts.foreign > 0 && (
              <p className="text-[12px] text-fg-1">
                {counts.foreign === 1
                  ? "1 file in the folder isn't SoundCheck's, skipped."
                  : `${counts.foreign} files in the folder aren't SoundCheck's, skipped.`}
              </p>
            )}
            {counts.otherTrack > 0 && (
              <p className="text-[12px] text-fg-1">
                {counts.otherTrack === 1
                  ? '1 file skipped: the folder has a copy of that name made from another file.'
                  : `${counts.otherTrack} files skipped: the folder has copies of those names made from other files.`}
              </p>
            )}
            {counts.sameName > 0 && (
              <p className="text-[12px] text-fg-1">
                {counts.sameName === 1
                  ? '1 file skipped: a file earlier in this export takes its name.'
                  : `${counts.sameName} files skipped: files earlier in this export take their names.`}
              </p>
            )}
            {preview.artefactsFolder && (
              <p className="text-[12px] leading-[1.45] text-fg-2">
                {inFolder && preview.xmlListed > 0
                  ? `The folder's rekordbox XML and grid report are updated; the ${plural(preview.xmlListed, 'track')} listed there stay${preview.xmlListed === 1 ? 's' : ''} listed. `
                  : 'The rekordbox XML and the grid report go to '}
                {!(inFolder && preview.xmlListed > 0) && (
                  <>
                    <span className="break-all font-mono text-fg-1">
                      {shortPath(preview.artefactsFolder)}
                    </span>
                    {preview.backupFolder ? ', in a folder named after this batch.' : '.'}
                  </>
                )}
              </p>
            )}
            {leftOut > 0 && (
              <p className="text-[12px] text-fg-2">
                {plural(leftOut, 'track')} not analysed {leftOut === 1 ? 'is' : 'are'} left out.
              </p>
            )}
            <Particulars rows={rows} />
          </>
        )}
        {phase !== 'failed' && phase !== 'planning' && differing.length > 0 && (
          <div className="flex flex-col gap-1.5 rounded-md border border-line bg-bg-2 px-3 py-2">
            {!again && <p className="text-[12.5px] text-fg-1">{stays}.</p>}
            <Toggle label={offer} checked={again} onChange={setExportAgain} />
          </div>
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
            disabled={phase !== 'ready' || !canWrite || !idle}
            onClick={confirmExport}
            className={`${buttonClass} border-transparent bg-accent text-bg-0`}
          >
            {ask}
          </button>
        )}
      </div>
    </Modal>
  );
}
