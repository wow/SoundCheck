import { Channel, invoke } from '@tauri-apps/api/core';
import type { AnalyzeRequest } from './generated/AnalyzeRequest';
import type { DecideSettings } from './generated/DecideSettings';
import type { ExportRequest } from './generated/ExportRequest';
import type { FileEntry } from './generated/FileEntry';
import type { ForgetResult } from './generated/ForgetResult';
import type { JobEvent } from './generated/JobEvent';
import type { RecoveryStatus } from './generated/RecoveryStatus';
import type { Replan } from './generated/Replan';
import type { RevertResult } from './generated/RevertResult';
import type { SessionSnapshot } from './generated/SessionSnapshot';
import type { WriteXmlRequest } from './generated/WriteXmlRequest';
import type { XmlWritten } from './generated/XmlWritten';

/**
 * Typed wrappers over Tauri commands. Components never call `invoke` directly; they call store
 * actions, which call these. Types come from `./generated/` (written by `cargo test -p sc-core`).
 */

export type { AnalyzeRequest } from './generated/AnalyzeRequest';
export type { AnalysisSettings } from './generated/AnalysisSettings';
export type { Codec } from './generated/Codec';
export type { BatchMode } from './generated/BatchMode';
export type { Confidence } from './generated/Confidence';
export type { Cut } from './generated/Cut';
export type { DecideSettings } from './generated/DecideSettings';
export type { DjUnsafe } from './generated/DjUnsafe';
export type { ExportAction } from './generated/ExportAction';
export type { ExportArtefacts } from './generated/ExportArtefacts';
export type { ExportNotice } from './generated/ExportNotice';
export type { ExportRequest } from './generated/ExportRequest';
export type { ExportRow } from './generated/ExportRow';
export type { ExportSettings } from './generated/ExportSettings';
export type { ExportSkip } from './generated/ExportSkip';
export type { ExportVerification } from './generated/ExportVerification';
export type { FileEntry } from './generated/FileEntry';
export type { FileInfo } from './generated/FileInfo';
export type { ForgetResult } from './generated/ForgetResult';
export type { GridCheck } from './generated/GridCheck';
export type { GridCheckSkip } from './generated/GridCheckSkip';
export type { GainPlan } from './generated/GainPlan';
export type { IpcError } from './generated/IpcError';
export type { IpcErrorKind } from './generated/IpcErrorKind';
export type { JobEvent } from './generated/JobEvent';
export type { JobStage } from './generated/JobStage';
export type { LoudnessMode } from './generated/LoudnessMode';
export type { Plan } from './generated/Plan';
export type { PendingChange } from './generated/PendingChange';
export type { Place } from './generated/Place';
export type { Reason } from './generated/Reason';
export type { RecoveredChange } from './generated/RecoveredChange';
export type { RecoveryOutcome } from './generated/RecoveryOutcome';
export type { RecoveryStatus } from './generated/RecoveryStatus';
export type { ReviewReason } from './generated/ReviewReason';
export type { RowAnalysis } from './generated/RowAnalysis';
export type { RowGrid } from './generated/RowGrid';
export type { Replan } from './generated/Replan';
export type { RevertResult } from './generated/RevertResult';
export type { RowPlan } from './generated/RowPlan';
export type { SessionRow } from './generated/SessionRow';
export type { SessionSnapshot } from './generated/SessionSnapshot';
export type { SkipReason } from './generated/SkipReason';
export type { WriteXmlRequest } from './generated/WriteXmlRequest';
export type { XmlNotListed } from './generated/XmlNotListed';
export type { XmlOnlyReason } from './generated/XmlOnlyReason';
export type { XmlWritten } from './generated/XmlWritten';

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

/**
 * Starts exporting; resolves to the job id at once while events arrive on `onEvent`, `finished`
 * last. Rejects with `recoveryRunning` while the start-up recovery runs, with `listedTwice` or
 * `sameOutputName` for a file a request names twice, and with `invalidArgument` for settings out
 * of their limits. Cancel with `cancelJob`.
 */
export function exportFiles(req: ExportRequest, onEvent: (event: JobEvent) => void): Promise<number> {
  const channel = new Channel<JobEvent>();
  channel.onmessage = onEvent;
  return invoke<number>('export', { req, onEvent: channel });
}

/**
 * Puts a file's original back, analyses it and refreshes its row. Rejects with `nothingToUndo`
 * for a file with no change to revert and `fileChanged` for one edited since it was written.
 */
export function revertFile(fileId: number): Promise<RevertResult> {
  return invoke<RevertResult>('revert_file', { fileId });
}

/** Writes a rekordbox XML of analysed rows, without exporting them. */
export function writeRekordboxXml(req: WriteXmlRequest): Promise<XmlWritten> {
  return invoke<XmlWritten>('write_rekordbox_xml', { req });
}

/** Gives up on a change recovery left pending, keeping every file it left. */
export function forgetPending(txn: string): Promise<ForgetResult> {
  return invoke<ForgetResult>('forget_pending', { txn });
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

/** Empties the track list in the engine (running jobs are cancelled); files are not touched. */
export function clearSession(): Promise<void> {
  return invoke<void>('clear_session');
}

/** Everything the Rust session holds, for a window that reloads; running jobs are cancelled. */
export function restoreSession(): Promise<SessionSnapshot> {
  return invoke<SessionSnapshot>('restore_session');
}

/** The median S-P95 of the analysed rows, or null before any row has one. */
export function calibrationTarget(): Promise<number | null> {
  return invoke<number | null>('calibration_target');
}

/**
 * What the crash recovery run at start found: file changes a crash interrupted, finished or
 * rolled back, and those left pending; `running` until it ends.
 */
export function recoveryStatus(): Promise<RecoveryStatus> {
  return invoke<RecoveryStatus>('recovery_status');
}

export * from './grid';
