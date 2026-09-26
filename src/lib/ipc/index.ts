import { Channel, invoke } from '@tauri-apps/api/core';
import type { AnalyzeRequest } from './generated/AnalyzeRequest';
import type { DecideSettings } from './generated/DecideSettings';
import type { FileEntry } from './generated/FileEntry';
import type { JobEvent } from './generated/JobEvent';
import type { Replan } from './generated/Replan';
import type { SessionSnapshot } from './generated/SessionSnapshot';

/**
 * Typed wrappers over Tauri commands. Components never call `invoke` directly; they call store
 * actions, which call these. Types come from `./generated/` (written by `cargo test -p sc-core`).
 */

export type { AnalyzeRequest } from './generated/AnalyzeRequest';
export type { AnalysisSettings } from './generated/AnalysisSettings';
export type { Codec } from './generated/Codec';
export type { Confidence } from './generated/Confidence';
export type { DecideSettings } from './generated/DecideSettings';
export type { DjUnsafe } from './generated/DjUnsafe';
export type { FileEntry } from './generated/FileEntry';
export type { FileInfo } from './generated/FileInfo';
export type { GainPlan } from './generated/GainPlan';
export type { IpcError } from './generated/IpcError';
export type { JobEvent } from './generated/JobEvent';
export type { JobStage } from './generated/JobStage';
export type { LoudnessMode } from './generated/LoudnessMode';
export type { Plan } from './generated/Plan';
export type { Reason } from './generated/Reason';
export type { ReviewReason } from './generated/ReviewReason';
export type { RowAnalysis } from './generated/RowAnalysis';
export type { RowGrid } from './generated/RowGrid';
export type { Replan } from './generated/Replan';
export type { RowPlan } from './generated/RowPlan';
export type { SessionRow } from './generated/SessionRow';
export type { SessionSnapshot } from './generated/SessionSnapshot';
export type { SkipReason } from './generated/SkipReason';

/** The application version as reported by the Rust side. */
export function appVersion(): Promise<string> {
  return invoke<string>('app_version');
}

/** Adds the audio files under `paths` (folders are walked); resolves to the new rows only. */
export function expandPaths(paths: string[]): Promise<FileEntry[]> {
  return invoke<FileEntry[]>('expand_paths', { paths });
}

/**
 * Starts analysing; resolves to the job id at once while events arrive on `onEvent`, `finished`
 * last.
 */
export function analyze(req: AnalyzeRequest, onEvent: (event: JobEvent) => void): Promise<number> {
  const channel = new Channel<JobEvent>();
  channel.onmessage = onEvent;
  return invoke<number>('analyze', { req, onEvent: channel });
}

/** Asks a running job to stop; resolves to false when no such job is running. */
export function cancelJob(jobId: number): Promise<boolean> {
  return invoke<boolean>('cancel_job', { jobId });
}

/**
 * Replans every analysed row under new loudness settings; rejects with an `invalidArgument`
 * error when a value is outside its limits.
 */
export function setDecideSettings(settings: DecideSettings): Promise<Replan> {
  return invoke<Replan>('set_decide_settings', { settings });
}

/** Everything the Rust session holds, for a window that reloads; running jobs are cancelled. */
export function restoreSession(): Promise<SessionSnapshot> {
  return invoke<SessionSnapshot>('restore_session');
}

/** The median S-P95 of the analysed rows, or null before any row has one. */
export function calibrationTarget(): Promise<number | null> {
  return invoke<number | null>('calibration_target');
}
