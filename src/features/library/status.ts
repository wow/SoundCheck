import type { Row } from '@/state/library';

/** The Status column's text. */
export function statusLabel(row: Row): string {
  if (row.reverting) return 'Reverting';
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
      return 'Skipped';
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
