import { ConfidenceRing, MeterBadge } from '@/features/library/cells';
import { usePanels } from '@/state/panels';
import { fitTones } from './geometry';
import { TONE_TEXT, VERDICT, VERDICT_TONE } from './fitText';
import { meterText } from './meters';
import { useTrack } from './store';
import { Chip, Kbd } from './ui';

/**
 * The fit in one line above the waveform, in a window too narrow for the rail: confidence,
 * tempo, meter, P95 and max against the verdict limits, the verdict, and the rest behind
 * Details (`I`), a drawer with everything the rail shows.
 */
export function FitStrip() {
  const grid = useTrack((s) => s.grid);
  const open = usePanels((p) => p.details);
  const setOpen = usePanels((p) => p.setDetails);
  const tones = grid ? fitTones(grid.residualP95Ms, grid.residualMaxMs) : null;
  return (
    <div className="flex h-8 shrink-0 items-center gap-2.5 border-b border-line bg-bg-1 px-3 text-[12px] text-fg-2">
      {grid && tones ? (
        <>
          <ConfidenceRing confidence={grid.confidence} />
          <span>
            <span className="font-mono text-[14px] font-semibold tabular-nums text-fg-0">
              {grid.bpm.toFixed(2)}
            </span>{' '}
            BPM
          </span>
          <MeterBadge meter={meterText(grid.meter)} />
          <span>
            P95{' '}
            <span className={`font-mono tabular-nums ${TONE_TEXT[tones.p95]}`}>
              {grid.residualP95Ms.toFixed(0)} ms
            </span>
          </span>
          <span>
            max{' '}
            <span className={`font-mono tabular-nums ${TONE_TEXT[tones.max]}`}>
              {grid.residualMaxMs.toFixed(0)} ms
            </span>
          </span>
          <Chip tone={VERDICT_TONE[grid.verdict]} mono={false}>
            {VERDICT[grid.verdict]}
          </Chip>
        </>
      ) : (
        <span>No beats found. Type the BPM, then put bar 1 with D.</span>
      )}
      <div className="flex-1" />
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        title="Fit, meter, changes and keys · I"
        className="inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-md border border-line px-2 text-[12px] font-medium text-fg-1 hover:bg-bg-2"
      >
        Details
        <Kbd>I</Kbd>
      </button>
    </div>
  );
}
