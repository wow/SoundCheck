import { useLibrary, isLocked, type Row } from '@/state/library';
import { rowErrorCopy, sentence } from '@/lib/ipc';
import { useRecovery } from '@/state/recovery';
import { revert, revertBlock } from './actions';
import { GridCheckBadge } from './ExportCells';
import { gridCheckChip } from './gridCheck';
import { exportActionText, startText, verifiedText } from './text';
import { exportDifference, useExportNow, type ExportNow } from './work';

function Line({ children, tone = 'text-fg-2' }: { children: string; tone?: string }) {
  return <p className={`break-words text-[12px] leading-[1.4] ${tone}`}>{sentence(children)}</p>;
}

/** The lines a row's export has to say, in order, from what the row carries. */
function Told({ row, now }: { row: Row; now: ExportNow }) {
  const shown = row.export;
  const different = exportDifference(row, now)?.text;
  if (row.state === 'failed' && row.exportError) {
    return (
      <>
        <Line tone="font-medium text-err">Failed</Line>
        <Line tone="text-fg-1">{rowErrorCopy(row.exportError, row.entry.path)}</Line>
        <Line>{row.changedOnDisk ? 'Press Analyse to read it again.' : 'The file was not changed.'}</Line>
      </>
    );
  }
  if (!shown) return null;
  const text = exportActionText(shown, row.entry.info.codec);
  if (shown.action.type !== 'write') {
    return (
      <>
        <Line tone="font-medium text-fg-1">{text.main ?? ''}</Line>
        {shown.notes.map((note) => (
          <Line key={note} tone="text-fg-1">
            {note}
          </Line>
        ))}
      </>
    );
  }
  const start = startText(shown.action.cut);
  const change = [text.main, text.gain, text.cut, start].filter(Boolean).join(' · ');
  return (
    <>
      <Line tone="font-medium text-fg-1">{change}</Line>
      {different && <Line tone="text-fg-1">{different}</Line>}
      {shown.verification && <Line tone="text-fg-1">{verifiedText(shown.verification)}</Line>}
      {shown.gridCheck && (
        <div className="flex flex-wrap items-baseline gap-x-2">
          <GridCheckBadge check={shown.gridCheck} />
          <Line>{gridCheckChip(shown.gridCheck).text}</Line>
        </div>
      )}
      {shown.notes.map((note) => (
        <Line key={note}>{note}</Line>
      ))}
    </>
  );
}

/**
 * Everything about the selected row's export, under the table: rows are one height, and a skip's
 * reason or a grid check's numbers need more than a row has.
 */
export function ExportDetail() {
  const id = useLibrary((s) => s.selected);
  const row = useLibrary((s) => (s.selected === null ? undefined : s.rows[s.selected]));
  const locked = useLibrary((s) => s.selected !== null && isLocked(s.locked, s.selected));
  const recovering = useRecovery((r) => r.running);
  const now = useExportNow();
  if (id === null || !row) return null;
  const hasStory =
    row.state === 'failed' ||
    row.export !== undefined ||
    row.revertible ||
    row.revertError ||
    row.reverting ||
    row.revertNotes?.length ||
    row.xmlNotListed;
  if (!hasStory) return null;
  const block = revertBlock(row, locked, recovering);
  return (
    <section
      aria-label="Export details"
      className="flex max-h-[132px] min-h-0 shrink-0 gap-3 overflow-y-auto border-t border-line bg-bg-1 px-4 py-2.5"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <Told row={row} now={now} />
        {row.exportStale && <Line tone="text-fg-1">The grid changed since this export; export it again to write it.</Line>}
        {row.reverting && (
          <p role="status" className="text-[12px] text-fg-1">
            Reverting: restoring the original and analysing it again…
          </p>
        )}
        {row.revertError && <Line tone="text-err">{rowErrorCopy(row.revertError, row.entry.path)}</Line>}
        {row.revertNotes?.map((note) => (
          <Line key={note}>{note}</Line>
        ))}
        {row.xmlNotListed && <Line>{`Not in the rekordbox XML: ${row.xmlNotListed}`}</Line>}
      </div>
      {row.revertible && (
        <button
          type="button"
          disabled={block !== null}
          title={block ?? undefined}
          onClick={() => void revert(id)}
          className="inline-flex h-7 shrink-0 items-center self-start whitespace-nowrap rounded-md border border-line px-2.5 text-[12.5px] font-medium text-fg-0 hover:bg-bg-2 disabled:cursor-default disabled:opacity-45"
        >
          {row.reverting ? 'Reverting…' : 'Revert'}
        </button>
      )}
    </section>
  );
}
