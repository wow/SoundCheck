/**
 * The volume slider's taper: 0 dB at the right end, -60 dB near the left end and off at it.
 * Two straight runs in dB: the right 70 % of the travel spans -30..0 dB, where listening
 * happens, the next 26 % spans -60..-30 dB, and the last 4 % is off.
 */

export const VOLUME_MIN_DB = -60;
export const VOLUME_MAX_DB = 0;
/** The share of the travel at the left end that is off. */
const OFF_POS = 0.04;
const KNEE_POS = 0.3;
const KNEE_DB = -30;

/** The volume at slider position `pos` (0 left .. 1 right): dB to a tenth, or null for off. */
export function volumeAt(pos: number): number | null {
  const p = Math.min(1, Math.max(0, pos));
  if (p < OFF_POS) return null;
  const db =
    p >= KNEE_POS
      ? KNEE_DB + ((p - KNEE_POS) / (1 - KNEE_POS)) * (VOLUME_MAX_DB - KNEE_DB)
      : VOLUME_MIN_DB + ((p - OFF_POS) / (KNEE_POS - OFF_POS)) * (KNEE_DB - VOLUME_MIN_DB);
  return Math.round(db * 10) / 10 || 0;
}

/** The slider position of `db` (null: off), the inverse of [`volumeAt`]. */
export function volumePos(db: number | null): number {
  if (db === null) return 0;
  const d = Math.min(VOLUME_MAX_DB, Math.max(VOLUME_MIN_DB, db));
  return d >= KNEE_DB
    ? KNEE_POS + ((d - KNEE_DB) / (VOLUME_MAX_DB - KNEE_DB)) * (1 - KNEE_POS)
    : OFF_POS + ((d - VOLUME_MIN_DB) / (KNEE_DB - VOLUME_MIN_DB)) * (KNEE_POS - OFF_POS);
}

/** One key step of `step` dB from `db`: up from off starts at -60 dB, down past -60 is off. */
export function volumeStep(db: number | null, step: number): number | null {
  if (db === null) return step > 0 ? VOLUME_MIN_DB : null;
  const next = Math.round((db + step) * 10) / 10;
  if (next < VOLUME_MIN_DB) return null;
  return Math.min(VOLUME_MAX_DB, next) || 0;
}

/** `-12.0 dB`, or `Off`. */
export function volumeText(db: number | null): string {
  return db === null ? 'Off' : `${db.toFixed(1).replace('-', '−')} dB`;
}
