import type { BatchMode } from '@/lib/ipc';
import type { Row } from '@/state/library';
import type { SavedSettings } from '@/state/settings';

/** The folder a file sits in. */
export function folderOf(path: string): string {
  const at = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return at > 0 ? path.slice(0, at) : path;
}

export interface BatchChoice {
  /** The mode the batch runs in. */
  mode: BatchMode;
  /** Prepare was chosen, but files with Serato data switched the batch to Library. */
  auto: boolean;
  /** Prepare kept for a batch with Serato data, as the user chose for these folders. */
  kept: boolean;
  seratoTracks: number;
  /** The folders those tracks sit in. */
  seratoFolders: string[];
}

/**
 * The batch's mode. Serato stores cue points as positions in the audio, so a cut would move them:
 * any such track makes the batch Library, unless the user chose Prepare for its folder. The choice
 * is kept per folder, and the saved mode is left alone, so the next batch of new tracks is Prepare
 * again.
 */
export function batchChoice(
  s: Pick<SavedSettings, 'batchMode' | 'seratoPrepareFolders'>,
  rows: Row[],
): BatchChoice {
  const serato = rows.filter((r) => r.entry.info.serato);
  const seratoFolders = [...new Set(serato.map((r) => folderOf(r.entry.path)))];
  const undecided = seratoFolders.some((f) => !s.seratoPrepareFolders.includes(f));
  const prepare = s.batchMode === 'prepare';
  return {
    mode: prepare && undecided ? 'library' : s.batchMode,
    auto: prepare && undecided,
    kept: prepare && !undecided && serato.length > 0,
    seratoTracks: serato.length,
    seratoFolders,
  };
}
