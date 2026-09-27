import type { TrackState } from './store';
import { clockText, groupOf, nearestLine } from './geometry';

/**
 * The texts that follow the playhead or the pointer. Written straight into the DOM by their
 * components: they change up to 30 times a second, which React re-renders should not.
 */

/** `0:32.418` and `17.2` (bar and beat or pulse of the nearest line), for the transport. */
export function playText(t: Pick<TrackState, 'opened' | 'grid' | 'player'>): {
  clock: string;
  bar: string;
} {
  const rate = t.opened?.sampleRate ?? 44_100;
  const at = t.player.position;
  const line = t.grid ? nearestLine(t.grid, rate, at) : null;
  return { clock: clockText(at, rate), bar: line ? `${line.bar}.${line.pulse}` : '–' };
}

/**
 * Time, bar and beat (or pulse and group), residual and short-term loudness at the pointer, or
 * at the playhead when the pointer is away (the start of a track just opened); null before the
 * track is open.
 */
export function readoutText(
  t: Pick<TrackState, 'opened' | 'grid' | 'fit' | 'player'>,
  hover: number | null,
): string | null {
  const at = hover ?? t.player.position;
  if (!t.opened || at === null) return null;
  const rate = t.opened.sampleRate;
  const parts = [clockText(at, rate)];
  if (t.grid) {
    const line = nearestLine(t.grid, rate, at);
    const where = `bar ${line.bar} ${t.grid.meter.unit === 'quarter' ? 'beat' : 'pulse'} ${line.pulse}`;
    const grouped = t.grid.meter.grouping.some((g) => g > 1);
    parts.push(grouped ? `${where} (group ${groupOf(t.grid.meter, line.pulse)})` : where);
    const r = t.fit ? t.fit.residuals[line.index - t.fit.header.firstLine] : undefined;
    if (r !== undefined && !Number.isNaN(r)) {
      const ms = Math.round(r) || 0; // never "-0"
      parts.push(`residual ${ms > 0 ? '+' : ''}${ms} ms`);
    }
  }
  const hop = t.opened.timeline.hopMs;
  const lufs = t.opened.timeline.shortTerm[Math.floor(((at / rate) * 1000) / hop)];
  if (lufs !== undefined && lufs !== null)
    parts.push(`${lufs.toFixed(1).replace('-', '−')} LUFS-S`);
  return parts.join(' · ');
}
