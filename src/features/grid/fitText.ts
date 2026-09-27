import type { Verdict } from '@/lib/ipc';
import type { ResidualTone } from './geometry';
import type { ChipTone } from './ui';

/** How the grid view names and colours a fit, in the rail and in the narrow fit strip. */

export const VERDICT: Record<Verdict, string> = {
  static: 'Static',
  staticWarn: 'Static, check by ear',
  drifts: 'Drifts',
};
export const VERDICT_TONE: Record<Verdict, ChipTone> = {
  static: 'ok',
  staticWarn: 'warn',
  drifts: 'err',
};
export const TONE_TEXT: Record<ResidualTone, string> = {
  ok: 'text-ok',
  warn: 'text-warn',
  err: 'text-err',
};
/** What the colours say, for screen readers. */
export const TONE_WORDS: Record<ResidualTone, string> = {
  ok: ', within the limit',
  warn: ', near the limit',
  err: ', over the limit',
};
