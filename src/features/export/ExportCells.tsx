import { Loader2, Undo2 } from 'lucide-react';
import type { ExportRow, GridCheck } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { isLocked, useLibrary, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { revert, revertBlock } from './actions';
import { gridCheckChip, type ChipTone } from './gridCheck';
import { exportActionText, verifiedText } from './text';
import { exportDifference, type ExportNow } from './work';

/** Tokens per tone. `neutral` must never read as a failure: no warning or error colour in it. */
const CHIP: Record<ChipTone, string> = {
  positive: 'text-ok',
  neutral: 'text-fg-1',
  attention: 'rounded border border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.12)] px-1 text-warn',
  quiet: 'text-warn/80',
  muted: 'text-fg-2',
};

/** A written file's grid check on one line; the whole sentence on hover and in the details. */
export function GridCheckBadge({ check }: { check: GridCheck }) {
  const chip = gridCheckChip(check);
  return (
    <span
      data-tone={chip.tone}
      title={chip.text}
      className={cn('block max-w-full truncate text-[12px] leading-4', CHIP[chip.tone])}
    >
      {chip.label}
    </span>
  );
}

const POSITIONS_SHIFT =
  'Positions shift: this file starts later in its audio, so cue points saved elsewhere sit late by the cut.';

/** The file's start was cut, so positions kept outside it no longer line up. */
export function PositionsShiftBadge() {
  return (
    <span
      title={POSITIONS_SHIFT}
      aria-label={POSITIONS_SHIFT}
      className="shrink-0 rounded border border-line px-1 font-sans text-[11px] leading-4 text-fg-1"
    >
      shift
    </span>
  );
}

/**
 * The Action column of a row with an export. `stacked`: too narrow for gain and cut on one line,
 * so they take the two lines, and what was verified moves to the row's details.
 */
export function ExportActionCell({
  row,
  exportRow,
  stacked,
  now,
}: {
  row: Row;
  exportRow: ExportRow;
  stacked: boolean;
  now?: ExportNow;
}) {
  const text = exportActionText(exportRow, row.entry.info.codec);
  const shift = exportRow.positionsShift;
  const verified = exportRow.verification ? verifiedText(exportRow.verification) : null;
  // Said first: the row's other numbers already are those of the settings now.
  const different = now ? exportDifference(row, now)?.text : undefined;
  const second = different ?? verified ?? text.detail;
  const main = text.main ?? [text.gain, text.cut].filter(Boolean).join(' · ');
  const twoChanges = text.gain !== null && text.cut !== null;
  const line = 'truncate font-mono text-xs font-medium';
  if (twoChanges && stacked && !different) {
    return (
      <div className="min-w-0" title={[main, second].filter(Boolean).join(' · ')}>
        <div className={line}>{text.gain}</div>
        <div className={cn('flex items-center gap-1.5', line)}>
          <span className="truncate">{text.cut}</span>
          {shift && <PositionsShiftBadge />}
        </div>
      </div>
    );
  }
  return (
    <div className="min-w-0">
      <div className="flex items-center gap-1.5">
        <span className={cn(line, text.tone === 'muted' && 'text-fg-2')} title={main}>
          {main}
        </span>
        {shift && <PositionsShiftBadge />}
      </div>
      {second && (
        <div className="truncate text-[12px] text-fg-2" title={second}>
          {second}
        </div>
      )}
    </div>
  );
}

/** Puts the file's original back; only on a row with a change to put back. */
export function RevertIcon({ id, row, selected }: { id: number; row: Row; selected: boolean }) {
  const locked = useLibrary((s) => isLocked(s.locked, id));
  const recovering = useRecovery((r) => r.running);
  if (!row.revertible || row.reverting) return null;
  const block = revertBlock(row, locked, recovering);
  return (
    <button
      type="button"
      aria-label="Revert to the original"
      title={block ?? 'Revert to the original'}
      disabled={block !== null}
      // One Tab stop for the table: the selected row's. The row's details hold the same action.
      tabIndex={selected ? 0 : -1}
      onClick={(e) => {
        e.stopPropagation();
        void revert(id);
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className="inline-flex size-[22px] shrink-0 items-center justify-center rounded-md text-fg-2 hover:bg-bg-3 hover:text-fg-0 disabled:opacity-45 disabled:hover:bg-transparent disabled:hover:text-fg-2"
    >
      <Undo2 className="size-3.5" aria-hidden="true" />
    </button>
  );
}

/** Shown on a row while its revert runs. */
export function RevertSpinner() {
  return <Loader2 className="size-3 motion-safe:animate-spin" aria-hidden="true" />;
}
