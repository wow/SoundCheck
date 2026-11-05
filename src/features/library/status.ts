import type { Row } from '@/state/library';

/** Its export finished, whatever its grid still asks: the reason shows under the pill. */
export function exportedAndRests(row: Row): boolean {
  return row.state === 'needsReview' && row.export?.stage === 'done';
}

/** Left to the rekordbox XML, which lists it: it carries 4/4 grids only. */
function listedInXml(row: Row): boolean {
  return row.export?.action.type === 'xmlOnly' && row.analysis?.grid?.fourFour !== false;
}

/** The Status column's text. */
export function statusLabel(row: Row): string {
  if (row.reverting) return 'Reverting';
  if (exportedAndRests(row)) return row.export?.verification ? 'Done, verified' : 'Done';
  switch (row.state) {
    case 'queued':
      return 'Queued';
    case 'analysing':
      return `Analysing ${Math.round(row.progress * 100)} %`;
    case 'analysed':
      return 'Analysed';
    case 'needsReview':
      return 'Needs review';
    case 'skipped':
      return listedInXml(row) ? 'XML only' : 'Skipped';
    case 'error':
      return 'Error';
    case 'cancelled':
      return 'Cancelled';
    case 'exportQueued':
      return 'Queued';
    case 'processing':
      return `Processing ${Math.round(row.progress * 100)} %`;
    case 'writing':
      return 'Writing';
    // Already verified: only the grid check is still running.
    case 'checking':
      return 'Checking';
    case 'done':
      return row.export?.verification ? 'Done, verified' : 'Done';
    case 'failed':
      return 'Failed';
  }
}
