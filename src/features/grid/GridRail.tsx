import { useMemo } from 'react';
import type { Grid, Verdict } from '@/lib/ipc';
import { REASON_TEXT } from '@/lib/format';
import { useLibrary } from '@/state/library';
import { ConfidenceRing, MeterBadge } from '@/features/library/cells';
import { chooseMeter, octave } from './actions';
import { changes } from './edit';
import { type ResidualTone, barOf, fitTones } from './geometry';
import { meterText } from './meters';
import { queuePlace, reviewQueue } from './navigate';
import { useTrack } from './store';
import { Chip, type ChipTone, Kbd } from './ui';

const VERDICT: Record<Verdict, string> = {
  static: 'Static',
  staticWarn: 'Static, check by ear',
  drifts: 'Drifts',
};
const VERDICT_TONE: Record<Verdict, ChipTone> = { static: 'ok', staticWarn: 'warn', drifts: 'err' };
const TONE_TEXT: Record<ResidualTone, string> = {
  ok: 'text-ok',
  warn: 'text-warn',
  err: 'text-err',
};

const CONFIDENCE_LINE = {
  green: 'Grid confirmed against the kick onsets',
  amber: 'Confirm bar 1 with the click',
  red: 'Check the tempo and bar 1 by ear',
} as const;

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[12.5px]">
      <span className="text-fg-2">{label}</span>
      <span className="text-right font-mono tabular-nums text-fg-0">{children}</span>
    </div>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section
      aria-label={title}
      className="flex flex-col gap-2 rounded-lg border border-line bg-bg-0 p-3"
    >
      <h2 className="text-[12px] font-semibold text-fg-1">{title}</h2>
      {children}
    </section>
  );
}

/** What the grid fit found, what was changed, and whether it is saved and confirmed. */
export function GridRail() {
  const grid = useTrack((s) => s.grid);
  const opened = useTrack((s) => s.opened);
  const fit = useTrack((s) => s.fit);
  const edit = useTrack((s) => s.edits.present);
  const fileId = useTrack((s) => s.fileId);
  const confirmed = useTrack((s) => s.confirmed);
  const saving = useTrack((s) => s.saving);
  const saveError = useTrack((s) => s.saveError);
  const refitNote = useTrack((s) => s.refitNote);
  const retrySave = useTrack((s) => s.retrySave);
  const order = useLibrary((s) => s.order);
  const rows = useLibrary((s) => s.rows);
  const queue = useMemo(() => reviewQueue(order, rows), [order, rows]);
  const place = fileId === null ? null : queuePlace(queue, fileId);
  const rate = opened?.sampleRate ?? 44_100;
  const chips = opened ? changes(opened.analysed, grid, edit, rate) : [];
  return (
    <aside className="flex w-[300px] shrink-0 flex-col gap-3 overflow-y-auto border-l border-line bg-bg-1 p-3.5">
      <div className="text-[12.5px] text-fg-2">
        {place ? (
          <span className="text-warn">
            Needs review {place.index} of {place.total}
          </span>
        ) : (
          `${queue.length} ${queue.length === 1 ? 'track needs' : 'tracks need'} review`
        )}
      </div>
      {refitNote && (
        <p
          role="status"
          className="rounded-md border border-[rgba(251,191,36,.45)] bg-[rgba(251,191,36,.08)] px-2.5 py-2 text-[12px] leading-snug text-warn"
        >
          {refitNote}
        </p>
      )}
      {grid ? (
        <FitCard
          grid={grid}
          matched={fit?.header.matched}
          attacks={fit?.header.attacks}
          worst={fit?.header.worstLine ?? null}
        />
      ) : (
        <NoGrid />
      )}
      {grid && !grid.meter.grouping.every((g) => g === 1) && <MeterCard grid={grid} />}
      <Card title="Changes">
        {chips.length === 0 ? (
          <span className="text-[12px] text-fg-2">As analysed.</span>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {chips.map((c) => (
              <Chip key={c} tone="amber">
                {c}
              </Chip>
            ))}
          </div>
        )}
      </Card>
      {saveError && !saving ? (
        <div
          role="alert"
          className="flex flex-col items-start gap-1.5 text-[12px] leading-snug text-err"
        >
          <span>Not saved: {saveError}</span>
          <button
            type="button"
            onClick={retrySave}
            className="h-7 rounded-md border border-line px-2.5 text-[12px] font-medium text-fg-1 hover:bg-bg-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2"
          >
            Save again
          </button>
        </div>
      ) : (
        <p aria-live="polite" className="text-[12px] leading-snug text-fg-2">
          {saving
            ? 'Saving…'
            : confirmed
              ? 'Confirmed by ear. This grid is saved and needs no review.'
              : 'Saved. Listen with the click, then press ⏎ to confirm.'}
        </p>
      )}
      <div className="mt-auto flex flex-col gap-1 text-[11.5px] text-fg-2">
        <span>
          <Kbd>⏎</Kbd> accept &amp; next · <Kbd>N</Kbd> next needs review
        </span>
        <span>
          <Kbd>Space</Kbd> play · <Kbd>C</Kbd> click · <Kbd>D</Kbd> beat 1 here
        </span>
        <span>
          <Kbd>,</Kbd> <Kbd>.</Kbd> nudge 1 ms · <Kbd>⇧</Kbd> 10 ms · <Kbd>⌥</Kbd> 1 beat
        </span>
        <span>
          <Kbd>1</Kbd>–<Kbd>{Math.min(9, grid?.meter.beatsPerBar ?? 4)}</Kbd> which{' '}
          {grid?.meter.unit === 'eighth' ? 'pulse' : 'beat'} is beat 1 · <Kbd>⌘Z</Kbd> undo
        </span>
      </div>
    </aside>
  );
}

function FitCard({
  grid,
  matched,
  attacks,
  worst,
}: {
  grid: Grid;
  matched: number | undefined;
  attacks: number | undefined;
  worst: number | null;
}) {
  const bpb = Math.max(1, grid.meter.beatsPerBar);
  const tones = fitTones(grid.residualP95Ms, grid.residualMaxMs);
  const up = grid.alternatives.octaveUp;
  const down = grid.alternatives.octaveDown;
  return (
    <Card title="Grid fit">
      <div className="flex items-baseline gap-2">
        <span className="font-mono text-[28px] font-semibold tabular-nums leading-none text-fg-0">
          {grid.bpm.toFixed(2)}
        </span>
        <span className="text-[12px] text-fg-2">BPM</span>
        <span className="ml-auto">
          <MeterBadge meter={meterText(grid.meter)} />
        </span>
      </div>
      <div className="flex items-center gap-2 text-[12.5px] text-fg-1">
        <ConfidenceRing confidence={grid.confidence} />
        {CONFIDENCE_LINE[grid.confidence]}
      </div>
      {grid.reasons.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {grid.reasons.map((r) => (
            <Chip key={r} tone={r === 'drifts' ? 'err' : 'warn'} mono={false}>
              {REASON_TEXT[r]}
            </Chip>
          ))}
        </div>
      )}
      <div className="h-px bg-line" />
      <Row label="Octave">
        <span className="flex gap-1.5">
          {down !== null && (
            <button
              type="button"
              className="rounded px-1 hover:bg-bg-2 hover:text-accent-2"
              onClick={() => octave(-1)}
            >
              ½ · {down.toFixed(2)}
            </button>
          )}
          {up !== null && (
            <button
              type="button"
              className="rounded px-1 hover:bg-bg-2 hover:text-accent-2"
              onClick={() => octave(1)}
            >
              ×2 · {up.toFixed(2)}
            </button>
          )}
        </span>
      </Row>
      {grid.meter.unit === 'eighth' && <Row label="Pulse">♪ {Math.round(grid.bpm)} / min</Row>}
      {/* Coloured by the limits the verdict below comes from. */}
      <Row label="Residual P95">
        <span className={TONE_TEXT[tones.p95]}>{grid.residualP95Ms.toFixed(0)} ms</span>
      </Row>
      <Row label="Residual max">
        <span className={TONE_TEXT[tones.max]}>
          {grid.residualMaxMs.toFixed(0)} ms
          {/* Which bar only matters when the worst line is off by a millisecond or more. */}
          {worst !== null && grid.residualMaxMs >= 0.5 ? ` · bar ${barOf(worst, bpb)}` : ''}
        </span>
      </Row>
      <Row label="Drift">
        <span className={grid.verdict === 'drifts' ? 'text-warn' : undefined}>
          {grid.driftPpm > 0 ? '+' : ''}
          {grid.driftPpm.toFixed(0)} ppm
        </span>
      </Row>
      {matched !== undefined && attacks !== undefined && (
        <Row label="Kick onsets matched">
          {matched.toLocaleString('en-US').replace(/,/g, ' ')} /{' '}
          {attacks.toLocaleString('en-US').replace(/,/g, ' ')}
        </Row>
      )}
      <Row label="Verdict">
        <Chip tone={VERDICT_TONE[grid.verdict]} mono={false}>
          {VERDICT[grid.verdict]}
        </Chip>
      </Row>
    </Card>
  );
}

function MeterCard({ grid }: { grid: Grid }) {
  const runnerUp = grid.meterRunnerUp;
  return (
    <Card title="Meter">
      <p className="text-[12.5px] leading-snug text-fg-1">
        {meterText(grid.meter)}: beat 1 is accented in the click, the start of each other group
        lightly.
      </p>
      {runnerUp && (
        <button
          type="button"
          onClick={() => chooseMeter(runnerUp)}
          className="self-start rounded-md border border-line px-2.5 py-1 font-mono text-[12px] text-fg-1 hover:bg-bg-2 hover:text-fg-0"
        >
          Try {meterText(runnerUp)}
        </button>
      )}
    </Card>
  );
}

function NoGrid() {
  return (
    <Card title="Grid fit">
      <p className="text-[12.5px] leading-snug text-fg-1">
        No beats were found. Type the BPM, then put bar 1 on the first downbeat with <Kbd>D</Kbd>.
      </p>
    </Card>
  );
}
