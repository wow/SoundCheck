import type { Row } from '@/state/library';

/**
 * The Needs review queue as the grid view walks it: rows needing review in table order, where
 * the open row stands in it, and which row `N` and "accept & next" open.
 */

export function reviewQueue(order: number[], rows: Record<number, Row>): number[] {
  return order.filter((id) => rows[id]?.state === 'needsReview');
}

/** The first row needing review after `current` in table order, wrapping; null when none. */
export function nextReview(
  order: number[],
  rows: Record<number, Row>,
  current: number,
): number | null {
  const at = order.indexOf(current);
  for (let k = 1; k <= order.length; k++) {
    const id = order[(at + k + order.length) % order.length];
    if (id !== undefined && id !== current && rows[id]?.state === 'needsReview') return id;
  }
  return null;
}

/** `current`'s place in the queue, 1-based, or null when it is not in it. */
export function queuePlace(
  queue: number[],
  current: number,
): { index: number; total: number } | null {
  const i = queue.indexOf(current);
  return i < 0 ? null : { index: i + 1, total: queue.length };
}
