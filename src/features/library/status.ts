import type { Row } from '@/state/library';

/** The Status column's text. */
export function statusLabel(row: Row): string {
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
  }
}
