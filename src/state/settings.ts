import { create } from 'zustand';
import type { AnalysisSettings, DecideSettings, LoudnessMode } from '@/lib/ipc';

/** A loudness preset. `djTarget` reads its target from the persisted DJ target. */
export type PresetId = 'djTarget' | 'clubHot' | 'spotify' | 'apple' | 'custom';

export interface Preset {
  id: Exclude<PresetId, 'custom'>;
  label: string;
  /** Fixed target, LUFS; `null` for the DJ target, which the user owns. */
  target: number | null;
  ceiling: number;
}

export const PRESETS: Record<LoudnessMode, Preset[]> = {
  dj: [
    { id: 'djTarget', label: 'DJ target', target: null, ceiling: -0.5 },
    { id: 'clubHot', label: 'Club hot', target: -8, ceiling: -0.5 },
  ],
  streaming: [
    { id: 'spotify', label: 'Spotify', target: -14, ceiling: -1 },
    { id: 'apple', label: 'Apple Music', target: -16, ceiling: -1 },
  ],
};

/** The DJ target until the user measures or calibrates their own. */
export const DEFAULT_DJ_TARGET = -11;

/** Limits the engine enforces too (`sc_core::plan`): targets, ceilings and BPM ranges. */
export const TARGET_RANGE: [number, number] = [-30, -4];
export const CEILING_RANGE: [number, number] = [-6, 0];
export const BPM_LIMITS: [number, number] = [40, 300];

/** The shape of the saved settings; bumped when it changes. */
export const SETTINGS_SCHEMA = 1;

/** The settings that are saved between runs. */
export interface SavedSettings {
  mode: LoudnessMode;
  preset: PresetId;
  djTarget: number;
  target: number;
  ceiling: number;
  bpmRange: [number, number];
}

export const DEFAULT_SETTINGS: SavedSettings = {
  mode: 'dj',
  preset: 'djTarget',
  djTarget: DEFAULT_DJ_TARGET,
  target: DEFAULT_DJ_TARGET,
  ceiling: -0.5,
  bpmRange: [70, 180],
};

interface SettingsState extends SavedSettings {
  /** True once the saved settings were read (or found missing). */
  hydrated: boolean;
  hydrate(saved: Partial<SavedSettings>): void;
  setMode(mode: LoudnessMode): void;
  setPreset(id: Exclude<PresetId, 'custom'>): void;
  setTarget(lufs: number): void;
  setCeiling(dbtp: number): void;
  setBpmRange(range: [number, number]): void;
  /** Sets the DJ target (from "Calibrate from my library") and selects it. */
  calibrate(lufs: number): void;
}

function presetValues(p: Preset, djTarget: number) {
  return { preset: p.id, target: p.target ?? djTarget, ceiling: p.ceiling };
}

/** Rounds a user-entered level to the tenths the fields show. */
function tenth(value: number): number {
  return Math.round(value * 10) / 10;
}

function clamp(value: number, [lo, hi]: [number, number]): number {
  return Math.min(hi, Math.max(lo, value));
}

function validRange(range: unknown): range is [number, number] {
  if (!Array.isArray(range) || range.length !== 2) return false;
  const [lo, hi] = range as unknown[];
  return (
    typeof lo === 'number' &&
    typeof hi === 'number' &&
    lo >= BPM_LIMITS[0] &&
    hi <= BPM_LIMITS[1] &&
    lo < hi
  );
}

/** The fields of `saved` that are present and valid; anything else keeps its default. */
export function sanitize(saved: Record<string, unknown>): Partial<SavedSettings> {
  const out: Partial<SavedSettings> = {};
  const num = (v: unknown, range: [number, number]) =>
    typeof v === 'number' && Number.isFinite(v) && v >= range[0] && v <= range[1];
  if (saved.mode === 'dj' || saved.mode === 'streaming') out.mode = saved.mode;
  const presets: unknown[] = ['djTarget', 'clubHot', 'spotify', 'apple', 'custom'];
  if (presets.includes(saved.preset)) out.preset = saved.preset as PresetId;
  if (num(saved.djTarget, TARGET_RANGE)) out.djTarget = saved.djTarget as number;
  if (num(saved.target, TARGET_RANGE)) out.target = saved.target as number;
  if (num(saved.ceiling, CEILING_RANGE)) out.ceiling = saved.ceiling as number;
  if (validRange(saved.bpmRange)) out.bpmRange = saved.bpmRange;
  return out;
}

export const useSettings = create<SettingsState>()((set, get) => ({
  ...DEFAULT_SETTINGS,
  hydrated: false,
  hydrate(saved) {
    set({ ...DEFAULT_SETTINGS, ...saved, hydrated: true });
  },
  setMode(mode) {
    if (mode === get().mode) return;
    const first = PRESETS[mode][0];
    if (!first) return;
    set({ mode, ...presetValues(first, get().djTarget) });
  },
  setPreset(id) {
    const p = PRESETS[get().mode].find((x) => x.id === id);
    if (p) set(presetValues(p, get().djTarget));
  },
  setTarget(lufs) {
    if (!Number.isFinite(lufs)) return;
    const target = tenth(clamp(lufs, TARGET_RANGE));
    if (get().preset === 'djTarget') set({ target, djTarget: target });
    else set({ target, preset: 'custom' });
  },
  setCeiling(dbtp) {
    if (!Number.isFinite(dbtp)) return;
    set({ ceiling: tenth(clamp(dbtp, CEILING_RANGE)), preset: 'custom' });
  },
  setBpmRange(range) {
    if (validRange(range)) set({ bpmRange: [Math.round(range[0]), Math.round(range[1])] });
  },
  calibrate(lufs) {
    const djTarget = tenth(clamp(lufs, TARGET_RANGE));
    set({ mode: 'dj', preset: 'djTarget', djTarget, target: djTarget, ceiling: -0.5 });
  },
}));

/** What the engine decides every row with. */
export function decideSettings(s: SavedSettings): DecideSettings {
  return { mode: s.mode, target: s.target, ceiling: s.ceiling, bpmRange: s.bpmRange };
}

/** What a new analysis job runs with. */
export function analysisSettings(s: SavedSettings): AnalysisSettings {
  return { bpmRange: s.bpmRange, grid: true, model: 'small' };
}

/** The saved part of the state. */
export function savedPart(s: SavedSettings): SavedSettings {
  return {
    mode: s.mode,
    preset: s.preset,
    djTarget: s.djTarget,
    target: s.target,
    ceiling: s.ceiling,
    bpmRange: s.bpmRange,
  };
}

/** The label the preset picker shows. */
export function presetLabel(s: SavedSettings): string {
  if (s.preset === 'custom') return 'Custom';
  return PRESETS[s.mode].find((p) => p.id === s.preset)?.label ?? 'Custom';
}
