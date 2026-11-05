import type { GridCheck, GridCheckSkip, GridDisagreement } from '@/lib/ipc';
import { signed } from '@/lib/format';
import { meterText } from '@/features/grid/meters';

/**
 * How a chip looks. An export that succeeded is never made to look like a failure by its check:
 * `neutral` carries no warning colour at all, `attention` is amber, `quiet` only tints its text.
 */
export type ChipTone = 'positive' | 'neutral' | 'attention' | 'quiet' | 'muted';

export interface GridCheckChip {
  /** What the row shows. */
  label: string;
  tone: ChipTone;
  /** The whole sentence, with its numbers: the chip's hover and the row's details. */
  text: string;
}

function milliseconds(ms: number): string {
  const value = Math.abs(ms);
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ms`;
}

function direction(ms: number): string {
  return ms >= 0 ? 'later' : 'earlier';
}

function disagreementText(d: GridDisagreement): string {
  switch (d.kind) {
    case 'offBy':
      return `It puts the bar line ${milliseconds(d.offsetMs)} ${direction(d.offsetMs)}.`;
    case 'bpmDiffers':
      return `It reads ${d.found.toFixed(2)} BPM (${signed(d.bpmDiff, 2)}).`;
    case 'meterDiffers':
      return `It reads ${meterText(d.found)}.`;
    case 'noGridFound':
      return 'It found no grid.';
  }
}

function skipText(reason: GridCheckSkip): string {
  switch (reason.type) {
    case 'noGrid':
      return 'The file has no grid to check.';
    case 'gridWithheld':
      return 'Its grid needs review, so no grid was written to check.';
    case 'oddMeter':
      return `The check covers 4/4 only, and this is ${meterText(reason.meter)}.`;
    case 'xmlOnly':
      return 'Only the rekordbox XML carries its grid; the file was not written.';
    case 'notAnalysed':
      return 'The written file could not be analysed.';
  }
}

const REPORTS = 'The export succeeded; this check only reports.';

/** What a row shows for its grid check, by `result` alone; nothing here reads a message. */
export function gridCheckChip(check: GridCheck): GridCheckChip {
  switch (check.result) {
    case 'pass':
      return {
        label: 'Grid checks out',
        tone: 'positive',
        text: `The written file's own analysis agrees with the exported grid, within ${milliseconds(check.offsetMs)}.`,
      };
    case 'advisory':
      return {
        label: 'Reads differently',
        tone: 'neutral',
        text: `The analysis reads this differently; your confirmed grid was kept. ${disagreementText(check.disagreement)}`,
      };
    case 'checkBar1': {
      const beats = Math.abs(check.beats);
      const count = `${beats} ${beats === 1 ? 'beat' : 'beats'}`;
      return {
        label: 'Check bar 1',
        tone: 'attention',
        text: `The analysis puts bar 1 ${count} ${check.beats > 0 ? 'later' : 'earlier'} (${milliseconds(check.offsetMs)}). Listen to it in the grid view. ${REPORTS}`,
      };
    }
    case 'offBy':
      return {
        label: `Off by ${milliseconds(check.offsetMs)}`,
        tone: 'quiet',
        text: `The written file's own analysis puts the bar line ${milliseconds(check.offsetMs)} ${direction(check.offsetMs)}. ${REPORTS}`,
      };
    case 'bpmDiffers':
      return {
        label: `BPM ${signed(check.bpmDiff, 2)}`,
        tone: 'quiet',
        text: `The written file's own analysis reads ${check.found.toFixed(2)} BPM (${signed(check.bpmDiff, 2)}). ${REPORTS}`,
      };
    case 'meterDiffers':
      return {
        label: `Meter ${meterText(check.found)}`,
        tone: 'quiet',
        text: `The written file's own analysis reads ${meterText(check.found)}${check.exported ? `, exported as ${meterText(check.exported)}` : ''}. ${REPORTS}`,
      };
    case 'noGridFound':
      return {
        label: 'No grid found',
        tone: 'quiet',
        text: `The written file's own analysis found no grid. ${REPORTS}`,
      };
    case 'notChecked':
      return { label: 'Not checked', tone: 'muted', text: skipText(check.reason) };
  }
}
