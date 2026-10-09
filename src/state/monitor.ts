import { create } from 'zustand';
import { VOLUME_MAX_DB, VOLUME_MIN_DB, volumeStep } from '@/lib/volume';

/**
 * The player's monitor volume: what the user hears, for every track, remembered across
 * restarts. It scales the sound only; the meters, the planned gain and the files never change
 * with it.
 */

export const MONITOR_SCHEMA = 1;

export interface SavedMonitor {
  /** dB from -60 to 0; null when the slider is at its left end (off). */
  db: number | null;
  /** The speaker icon's mute, which keeps the slider where it is. */
  muted: boolean;
}

export const DEFAULT_MONITOR: SavedMonitor = { db: 0, muted: false };

/** Where unmuting a slider left at off brings it. */
const UNMUTE_FROM_OFF_DB = -20;

interface MonitorState extends SavedMonitor {
  hydrated: boolean;
  hydrate(saved: Partial<SavedMonitor>): void;
  setDb(db: number | null): void;
  /** `+1` / `-1` dB (Shift+Up / Shift+Down); unmutes. */
  step(step: number): void;
  toggleMute(): void;
}

/** The fields of `saved` that are present and valid. */
export function sanitizeMonitor(saved: Record<string, unknown>): Partial<SavedMonitor> {
  const out: Partial<SavedMonitor> = {};
  const db = saved.db;
  if (db === null) out.db = null;
  else if (
    typeof db === 'number' &&
    Number.isFinite(db) &&
    db >= VOLUME_MIN_DB &&
    db <= VOLUME_MAX_DB
  )
    out.db = db;
  if (typeof saved.muted === 'boolean') out.muted = saved.muted;
  return out;
}

/** What the player is told: dB, or null for silence (muted or off). */
export function heardDb(m: SavedMonitor): number | null {
  return m.muted ? null : m.db;
}

export const useMonitor = create<MonitorState>()((set, get) => ({
  ...DEFAULT_MONITOR,
  hydrated: false,
  hydrate(saved) {
    set({ ...DEFAULT_MONITOR, ...saved, hydrated: true });
  },
  setDb(db) {
    const clamped = db === null ? null : Math.min(VOLUME_MAX_DB, Math.max(VOLUME_MIN_DB, db));
    set({ db: clamped, muted: false });
  },
  step(step) {
    set({ db: volumeStep(get().db, step), muted: false });
  },
  toggleMute() {
    const { muted, db } = get();
    if (!muted && db === null) set({ db: UNMUTE_FROM_OFF_DB });
    else set({ muted: !muted });
  },
}));
