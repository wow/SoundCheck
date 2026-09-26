import type { Codec, Confidence } from '@/lib/ipc';
import {
  CODEC_LABEL,
  DJ_UNSAFE_TEXT,
  actionText,
  displayTitle,
  level,
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
    <svg width="14" height="14" viewBox="0 0 14 14" role="img" aria-label={`Grid confidence: ${r.label}`}>
      <circle cx="7" cy="7" r="5.5" fill={r.fill} stroke={r.stroke} strokeWidth="2" />
    </svg>
  );
}

export function MeterBadge({ meter }: { meter: string }) {
  return (
    <span className="inline-flex h-5 items-center whitespace-nowrap rounded-[5px] border border-[rgba(79,209,197,.35)] bg-[rgba(79,209,197,.10)] px-[7px] font-mono text-[10.5px] font-medium text-accent-2">
      {meter}
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

export function NameCell({ row }: { row: Row }) {
  const { info, path } = row.entry;
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <CodecBadge codec={info.codec} />
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
        <span className="text-warn" title={DJ_UNSAFE_TEXT[info.djUnsafe]} aria-label={DJ_UNSAFE_TEXT[info.djUnsafe]}>
          !
        </span>
      )}
    </span>
  );
}

export function LoudnessCell({ row, target }: { row: Row; target: number }) {
  const measured = row.plan?.measured;
  if (measured == null) return <Dash />;
  const delta = target - measured;
  const width = Math.min(Math.abs(delta) * 4, 28);
  return (
    <div className="flex items-center gap-2">
      <span className="font-mono text-[13px] font-medium">{level(measured)}</span>
      <span className="text-fg-2">→</span>
      <span className="font-mono text-[13px] font-medium text-fg-2">{level(target)}</span>
      <span className="relative inline-block h-1 w-[60px] overflow-hidden rounded-sm bg-bg-3" aria-hidden="true">
        <span
          className={cn('absolute top-0 h-full', isShort(row) ? 'bg-accent' : 'bg-accent-2')}
          style={delta >= 0 ? { left: 30, width } : { right: 30, width }}
        />
      </span>
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
      {level(tp)}
    </span>
  );
}

export function BpmCell({ row }: { row: Row }) {
  const grid = row.analysis?.grid;
  if (!grid) {
    if (row.analysis?.gridSkipped === 'no beats found') {
      return <span className="text-xs text-fg-2">no beats</span>;
    }
    return <Dash />;
  }
  const octave = grid.reasons.includes('octaveMargin');
  return (
    <div className="flex items-center gap-1.5 whitespace-nowrap">
      <ConfidenceRing confidence={grid.confidence} />
      <span className="font-mono text-[13px] font-medium">{grid.bpm.toFixed(2)}</span>
      {octave && (
        <span className="font-mono text-[11px] text-warn" title="Half or double the tempo fits almost as well">
          ÷2 ×2?
        </span>
      )}
      {!grid.fourFour && <MeterBadge meter={grid.meter} />}
      {grid.verdict === 'staticWarn' && (
        <span
          className="text-[11px] text-fg-2"
          title={`The grid fits, but some beats land up to ${Math.round(grid.residualMaxMs)} ms off it; worth a listen`}
        >
          check
        </span>
      )}
    </div>
  );
}

export function ActionCell({ row, bpmRange }: { row: Row; bpmRange: [number, number] }) {
  if (row.state === 'error') {
    return (
      <div className="min-w-0">
        <div className="truncate text-[12.5px] font-medium text-err" title={row.error?.message}>
          {row.error?.message ?? 'Could not analyse'}
        </div>
        <div className="truncate text-[11px] text-fg-2">Analyse again, or check the file</div>
      </div>
    );
  }
  if (!row.plan) return <Dash />;
  const action = actionText(row.plan);
  const review = reviewText(row.plan.review, row.analysis, bpmRange);
  const detail = [action.detail, review].filter(Boolean).join(' · ');
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
