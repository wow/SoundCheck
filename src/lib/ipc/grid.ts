import { Channel, invoke } from '@tauri-apps/api/core';
import type { GridEdit } from './generated/GridEdit';
import type { GridFitHeader } from './generated/GridFitHeader';
import type { RowUpdate } from './generated/RowUpdate';
import type { TrackEvent } from './generated/TrackEvent';
import type { TrackOpened } from './generated/TrackOpened';

/**
 * Typed wrappers over the grid view's commands. Waveform bins, residuals and onsets arrive as
 * raw little-endian bytes (`tauri::ipc::Response`) and are turned into typed arrays here.
 */

export type { FitChoice } from './generated/FitChoice';
export type { Grid } from './generated/Grid';
/** Which part of the track the grid is fitted to (named apart from a refit's answer, `GridFit`). */
export type { GridFit as EditFit } from './generated/GridFit';
export type { GridEdit } from './generated/GridEdit';
export type { GridFitHeader } from './generated/GridFitHeader';
export type { Meter } from './generated/Meter';
export type { RowUpdate } from './generated/RowUpdate';
export type { TrackEvent } from './generated/TrackEvent';
export type { TrackOpened } from './generated/TrackOpened';
export type { Verdict } from './generated/Verdict';

/** A refit's answer: the fit and one residual in milliseconds per grid line (NaN: no attack). */
export interface GridFit {
  header: GridFitHeader;
  residuals: Float32Array;
}

/** Opens a row in the grid view; decoding, analysis and player events arrive on `onEvent`. */
export function trackOpen(
  fileId: number,
  onEvent: (event: TrackEvent) => void,
): Promise<TrackOpened> {
  const channel = new Channel<TrackEvent>();
  channel.onmessage = onEvent;
  return invoke<TrackOpened>('track_open', { fileId, onEvent: channel });
}

/** Closes the grid view; the player stops and the track's audio is released. */
export function trackClose(): Promise<void> {
  return invoke<void>('track_close');
}

/** Min/max pairs of `bins` bins of `samplesPerBin` frames from `firstBin`. */
export async function readPeaks(
  fileId: number,
  samplesPerBin: number,
  firstBin: number,
  bins: number,
): Promise<Int16Array> {
  const bytes = await invoke<ArrayBuffer>('read_peaks', { fileId, samplesPerBin, firstBin, bins });
  return new Int16Array(bytes);
}

/** The open track's embedded cover, or null. */
export async function trackCover(fileId: number, mime: string): Promise<Blob | null> {
  const bytes = await invoke<ArrayBuffer>('track_cover', { fileId });
  return bytes.byteLength > 0 ? new Blob([bytes], { type: mime }) : null;
}

/** The attacks bar 1 snaps to, in seconds. */
export async function trackOnsets(fileId: number): Promise<Float64Array> {
  const bytes = await invoke<ArrayBuffer>('track_onsets', { fileId });
  return new Float64Array(bytes);
}

/**
 * Splits `grid_refit`'s bytes: a u32 header length, the JSON header, then f32 residuals, as
 * many as the header's `lines`. Throws on bytes that do not have that shape.
 */
export function parseGridFit(bytes: ArrayBuffer): GridFit {
  if (bytes.byteLength < 4)
    throw new Error(`grid fit: ${bytes.byteLength} bytes, no header length`);
  const length = new DataView(bytes).getUint32(0, true);
  const tail = bytes.byteLength - 4 - length;
  if (tail < 0 || tail % 4 !== 0) {
    throw new Error(
      `grid fit: ${bytes.byteLength} bytes do not hold a ${length}-byte header and f32 residuals`,
    );
  }
  const header = JSON.parse(
    new TextDecoder().decode(new Uint8Array(bytes, 4, length)),
  ) as GridFitHeader;
  const residuals = new Float32Array(bytes.slice(4 + length));
  if (residuals.length !== header.lines) {
    throw new Error(`grid fit: ${residuals.length} residuals for ${header.lines} lines`);
  }
  return { header, residuals };
}

/** The grid `edit` gives on the open track; nothing is saved, the click follows it. */
export async function gridRefit(fileId: number, edit: GridEdit): Promise<GridFit> {
  return parseGridFit(await invoke<ArrayBuffer>('grid_refit', { fileId, edit }));
}

/** Saves `edit`, confirmed or not; resolves to the row and its plan. */
export function gridCommit(fileId: number, edit: GridEdit, confirm: boolean): Promise<RowUpdate> {
  return invoke<RowUpdate>('grid_commit', { fileId, edit, confirm });
}

/** Plays from `from` (samples), or from where it stopped. */
export function playerPlay(from: number | null): Promise<void> {
  return invoke<void>('player_play', { from });
}

export function playerPause(): Promise<void> {
  return invoke<void>('player_pause');
}

export function playerSeek(to: number): Promise<void> {
  return invoke<void>('player_seek', { to });
}

export function playerSetClick(on: boolean): Promise<void> {
  return invoke<void>('player_set_click', { on });
}
