import type { Codec, Confidence } from '@/lib/ipc';
import {
  CODEC_LABEL,
  DJ_UNSAFE_TEXT,
  actionText,
  displayTitle,
  level,
  peak,
  reviewText,
  specText,
} from '@/lib/format';
import { cn } from '@/lib/utils';
import type { Row, RowState } from '@/state/library';
import { isShort } from '@/state/library';
import { statusLabel } from './status';

export function CodecBadge({ codec }: { codec: Codec }) {
  return (
    <span className="inline-flex h-[18px] w-[38px] shrink-0 items-center justify-center rounded border border-line bg-bg-2 font-mono text-[9.5px] font-semibold tracking-wide text-accent-2">
      {CODEC_LABEL[codec]}
    </span>
  );
}

const RING: Record<Confidence, { stroke: string; fill: string; label: string }> = {
  green: { stroke: 'var(--sc-ok)', fill: 'rgba(74,222,128,.18)', label: 'Confident' },
  amber: { stroke: 'var(--sc-warn)', fill: 'rgba(251,191,36,.18)', label: 'Check' },
  red: { stroke: 'var(--sc-err)', fill: 'rgba(248,113,113,.18)', label: 'Unsure' },
};

export function ConfidenceRing({ confidence }: { confidence: Confidence }) {
  const r = RING[confidence];
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 14 14"
      role="img"
      aria-label={`Grid confidence: ${r.label}`}
    >
      <circle cx="7" cy="7" r="5.5" fill={r.fill} stroke={r.stroke} strokeWidth="2" />
    </svg>
  );
}

/** The meter (`9/8 · 2+2+2+3`); `short` shows only the time signature, the rest on hover. */
export function MeterBadge({ meter, short = false }: { meter: string; short?: boolean }) {
  const shown = short ? (meter.split(' · ')[0] ?? meter) : meter;
  return (
    <span
      className="inline-flex h-5 items-center whitespace-nowrap rounded-[5px] border border-[rgba(79,209,197,.35)] bg-[rgba(79,209,197,.10)] px-[7px] font-mono text-[10.5px] font-medium text-accent-2"
      title={shown !== meter ? meter : undefined}
    >
      {shown}
    </span>
  );
}

const PILL: Record<RowState, string> = {
  queued: 'text-fg-2 border-line bg-transparent',
  analysing: 'text-accent-2 border-[rgba(79,209,197,.35)] bg-[rgba(79,209,197,.08)]',
  analysed: 'text-fg-1 border-line bg-bg-2',
  needsReview: 'text-warn border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.12)]',
  skipped: 'text-fg-2 border-line bg-transparent',
  error: 'text-err border-[rgba(248,113,113,.45)] bg-[rgba(248,113,113,.10)]',
  cancelled: 'text-fg-2 border-line bg-transparent',
};

export function StatusPill({ row }: { row: Row }) {
  return (
    <span
      className={cn(
        'inline-flex h-[22px] items-center whitespace-nowrap rounded-md border px-2 text-xs font-medium',
        PILL[row.state],
      )}
    >
      {statusLabel(row)}
    </span>
  );
}

function Dash() {
  return <span className="font-mono text-fg-2">—</span>;
}

export function NameCell({ row, flagUnsafe = false }: { row: Row; flagUnsafe?: boolean }) {
  const { info, path } = row.entry;
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <span className="relative">
        <CodecBadge codec={info.codec} />
        {flagUnsafe && info.djUnsafe && (
          <span
            className="absolute -right-1.5 -top-1.5 text-[11px] font-bold text-warn"
            title={DJ_UNSAFE_TEXT[info.djUnsafe]}
            aria-label={DJ_UNSAFE_TEXT[info.djUnsafe]}
          >
            !
          </span>
        )}
      </span>
      <div className="min-w-0">
        <div className="truncate font-medium" title={path}>
          {displayTitle(info, path)}
        </div>
        <div className="truncate text-[11px] text-fg-2">{info.artist ?? info.album ?? ''}</div>
      </div>
    </div>
  );
}

export function SpecCell({ row }: { row: Row }) {
  const { info } = row.entry;
  return (
    <span className="flex items-center gap-1 whitespace-nowrap font-mono text-xs font-medium text-fg-1">
      {specText(info)}
      {info.djUnsafe && (
        <span
          className="text-warn"
          title={DJ_UNSAFE_TEXT[info.djUnsafe]}
          aria-label={DJ_UNSAFE_TEXT[info.djUnsafe]}
        >
          !
        </span>
      )}
    </span>
  );
}

/**
 * Measured loudness → target, with the delta bar in the wide layout. Given `tpCeiling` (the
 * narrow layout, which has no TP column) the true peak goes on a second line.
 */
export function LoudnessCell({
  row,
  target,
  bar = true,
  tpCeiling,
}: {
  row: Row;
  target: number;
  bar?: boolean;
  tpCeiling?: number;
}) {
  const measured = row.plan?.measured;
  if (measured == null) return <Dash />;
  const delta = target - measured;
  const width = Math.min(Math.abs(delta) * 4, 28);
  const levels = (
    <div className="flex items-center gap-1.5">
      <span className="font-mono text-[13px] font-medium">{level(measured)}</span>
      <span className="text-fg-2">→</span>
      <span className="font-mono text-[13px] font-medium text-fg-2">{level(target)}</span>
      {bar && (
        <span
          className="relative inline-block h-1 w-[60px] overflow-hidden rounded-sm bg-bg-3"
          aria-hidden="true"
        >
          <span
            className={cn('absolute top-0 h-full', isShort(row) ? 'bg-accent' : 'bg-accent-2')}
            style={delta >= 0 ? { left: 30, width } : { right: 30, width }}
          />
        </span>
      )}
    </div>
  );
  if (tpCeiling === undefined) return levels;
  const tp = row.analysis?.truePeak;
  return (
    <div className="flex min-w-0 flex-col">
      {levels}
      {tp != null && (
        <span
          className={cn('font-mono text-[11px]', tp > tpCeiling ? 'text-err' : 'text-fg-2')}
          title={tp > tpCeiling ? `Above the ${tpCeiling.toFixed(1)} dBTP ceiling` : undefined}
        >
          TP {peak(tp)}
        </span>
      )}
    </div>
  );
}

export function TpCell({ row, ceiling }: { row: Row; ceiling: number }) {
  const tp = row.analysis?.truePeak;
  if (tp == null) return <Dash />;
  return (
    <span
      className={cn('font-mono text-[13px] font-medium', tp > ceiling ? 'text-err' : 'text-fg-0')}
      title={tp > ceiling ? `Above the ${ceiling.toFixed(1)} dBTP ceiling` : undefined}
    >
      {peak(tp)}
    </span>
  );
}

/**
 * The tempo with its confidence (or ✓ when confirmed by ear), an edited mark, and the flags: an
 * octave in doubt, a meter other than 4/4, a grid worth a listen. `compact` shortens the meter
 * to its time signature; `stacked` (the narrow layout) puts the flags on a second line.
 */
export function BpmCell({
  row,
  variant = 'wide',
}: {
  row: Row;
  variant?: 'wide' | 'compact' | 'stacked';
}) {
  const grid = row.analysis?.grid;
  if (!grid) {
    if (row.analysis?.gridSkipped === 'no beats found') {
      return <span className="text-xs text-fg-2">no beats</span>;
    }
    return <Dash />;
  }
  const octave = grid.reasons.includes('octaveMargin') && !row.analysis?.confirmed;
  const flags = (
    <>
      {octave && (
        <span
          className="font-mono text-[11px] text-warn"
          title="Half or double the tempo fits almost as well"
        >
          ÷2 ×2?
        </span>
      )}
      {!grid.fourFour && <MeterBadge meter={grid.meter} short={variant !== 'wide'} />}
      {grid.verdict === 'staticWarn' && (
        <span
          className="text-[11px] text-fg-2"
          title={`The grid fits, but some beats land up to ${Math.round(grid.residualMaxMs)} ms off it; worth a listen`}
        >
          check
        </span>
      )}
    </>
  );
  const flagged = octave || !grid.fourFour || grid.verdict === 'staticWarn';
  const tempo = (
    <div className="flex items-center gap-1.5 whitespace-nowrap">
      {row.analysis?.confirmed ? (
        <span
          className="w-3.5 text-center text-[13px] font-semibold text-ok"
          role="img"
          aria-label="Grid confirmed by ear"
          title="Grid confirmed by ear"
        >
          ✓
        </span>
      ) : (
        <ConfidenceRing confidence={grid.confidence} />
      )}
      <span className="font-mono text-[13px] font-medium">{grid.bpm.toFixed(2)}</span>
      {row.analysis?.edited && (
        <span
          className="size-1.5 rounded-full bg-accent"
          role="img"
          aria-label="Edited in the grid view"
          title="Edited in the grid view"
        />
      )}
      {variant !== 'stacked' && flags}
    </div>
  );
  if (variant !== 'stacked' || !flagged) return tempo;
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      {tempo}
      <div className="flex items-center gap-1.5 whitespace-nowrap">{flags}</div>
    </div>
  );
}

export function ActionCell({ row, bpmRange }: { row: Row; bpmRange: [number, number] }) {
  if (row.state === 'error') {
    const missing = row.error?.kind === 'io' && /no such file|not found/i.test(row.error.message);
    return (
      <div className="min-w-0">
        <div className="truncate text-[12.5px] font-medium text-err" title={row.error?.message}>
          {missing ? 'File not found' : (row.error?.message ?? 'Could not analyse')}
        </div>
        <div className="truncate text-[11px] text-fg-2">
          {missing ? 'Moved or deleted since it was added' : 'Analyse again, or check the file'}
        </div>
      </div>
    );
  }
  if (!row.plan) return <Dash />;
  const action = actionText(row.plan);
  const review = reviewText(row.plan.review, row.analysis, bpmRange);
  // What to check comes first: on a Needs review row it is why the row is there.
  const detail = [review, action.detail].filter(Boolean).join(' · ');
  return (
    <div className="min-w-0">
      <div
        className={cn(
          'truncate font-mono text-xs font-medium',
          action.tone === 'accent' && 'text-accent',
          action.tone === 'muted' && 'text-fg-2',
        )}
        title={action.main}
      >
        {action.main}
      </div>
      {detail && (
        <div className="truncate text-[11px] text-fg-2" title={detail}>
          {detail}
        </div>
      )}
    </div>
  );
}
