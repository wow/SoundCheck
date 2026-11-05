import type { Codec, Cut, ExportArtefacts, ExportRow, ExportVerification } from '@/lib/ipc';
import { CODEC_LABEL, clock, signed } from '@/lib/format';
import type { LastExport } from '@/state/library';

/** A gain smaller than this changes nothing audible, and the engine calls it no gain. */
export const NEGLIGIBLE_DB = 0.05;

/** `Verified: 27/27 carried, 3 added`: every tag block but those written anew (format, audio). */
export function verifiedText(v: ExportVerification): string {
  const total = v.blocks - v.replaced;
  const kept = v.carried + v.patched + v.edited;
  const parts = [
    total > 0 ? `${kept}/${total} carried` : 'no tags to carry',
    v.tagsAdded > 0 && `${v.tagsAdded} added`,
    v.dropped > 0 && `${v.dropped} dropped`,
  ].filter(Boolean);
  return `Verified: ${parts.join(', ')}`;
}

/** What the plan says about the start of a file when nothing is cut; `null` when it is cut. */
export function startText(cut: Cut): string | null {
  switch (cut.type) {
    case 'cut':
    case 'gridOnly':
      return null;
    case 'onBar':
      return cut.barLine === cut.bar1
        ? 'Starts on bar 1'
        : `Starts on a bar line (bar 1 at ${cut.bar1S.toFixed(2)} s)`;
    case 'notCut':
      return cut.bar1 === cut.firstBarLine
        ? `Not cut: bar 1 ${cut.bar1S.toFixed(2)} s in`
        : `Not cut: first bar line ${cut.firstBarLineS.toFixed(2)} s in (bar 1 at ${cut.bar1S.toFixed(2)} s)`;
    case 'needsReview':
      return 'Not cut: grid needs review';
    case 'noGrid':
      return 'Not cut: no grid';
    case 'library':
      return 'Length kept';
  }
}

export interface ExportActionText {
  /** The change to the audio: `Gain −2.3 dB`, `Cut 0.21 s`, each `null` when there is none. */
  gain: string | null;
  cut: string | null;
  /** Instead of those, when the audio does not change or the file is not written. */
  main: string | null;
  tone: 'fg' | 'muted';
  /** Why it is not written, in the row's own words; or what happens at its start. */
  detail: string | null;
}

/** The Action column of a row with an export. */
export function exportActionText(row: ExportRow, codec: Codec): ExportActionText {
  const note = row.notes[0] ?? null;
  const none = { gain: null, cut: null };
  switch (row.action.type) {
    case 'xmlOnly':
      return { ...none, main: `Grid: XML only (${CODEC_LABEL[codec]})`, tone: 'muted', detail: note };
    case 'skip':
      return { ...none, main: 'Skipped', tone: 'muted', detail: note };
    case 'write': {
      const { gainDb, cut } = row.action;
      if (cut.type === 'gridOnly') return { ...none, main: 'Grid only', tone: 'fg', detail: note };
      const gain = Math.abs(gainDb) >= NEGLIGIBLE_DB ? `Gain ${signed(gainDb)} dB` : null;
      const cutText = cut.type === 'cut' ? `Cut ${cut.seconds.toFixed(2)} s` : null;
      return {
        gain,
        cut: cutText,
        main: gain || cutText ? null : 'No gain',
        tone: 'fg',
        detail: startText(cut),
      };
    }
  }
}

/** The footer's summary of the last export. */
export function lastExportText(last: LastExport): string {
  const parts = [
    `${last.written} exported`,
    last.xmlOnly > 0 && `${last.xmlOnly} XML only`,
    last.skipped > 0 && `${last.skipped} skipped`,
    last.failed > 0 && `${last.failed} failed`,
  ].filter(Boolean);
  const time = last.seconds !== null ? ` in ${clock(last.seconds * 1000)}` : '';
  const end = last.stoppedByReload
    ? ' (stopped: the window reloaded)'
    : last.cancelled
      ? ' (cancelled)'
      : '';
  return `Last export: ${parts.join(', ')}${time}${end}`;
}

/** What the XML holds, said plainly: an empty one still is a file on disk. */
export function xmlListedText(artefacts: ExportArtefacts, leftOut: number): string {
  if (artefacts.listed === 0) return 'The rekordbox XML lists no tracks: none has a grid it may carry';
  const tracks = `${artefacts.listed} ${artefacts.listed === 1 ? 'track' : 'tracks'}`;
  return `rekordbox XML lists ${tracks}${leftOut > 0 ? `, ${leftOut} left out` : ''}`;
}
