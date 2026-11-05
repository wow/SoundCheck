import { errorCopy, ipcErrorOf, revertFile, writeRekordboxXml } from '@/lib/ipc';
import { displayTitle } from '@/lib/format';
import { chooseExportFolder, revealInFinder } from '@/lib/platform';
import { useTrack } from '@/features/grid/store';
import { pipeline } from '@/features/pipeline/actions';
import { isLocked, useLibrary, xmlRowIds, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';

/**
 * The grid view of a file about to change cannot stay open (the engine closes the player it plays
 * through). It closes here, with a word about where it went.
 */
export function closeGridViewOf(fileIds: number[], doing: string): void {
  const track = useTrack.getState();
  if (track.fileId === null || !fileIds.includes(track.fileId)) return;
  const row = useLibrary.getState().rows[track.fileId];
  const name = row ? displayTitle(row.entry.info, row.entry.path) : 'That track';
  void track.close();
  useLibrary.getState().setInfo(`The grid view closed: ${name} is ${doing}. Open it again once that is done.`);
}

/** Why Revert cannot run on `row` now; `null` when it can. */
export function revertBlock(row: Row, locked: boolean, recoveryRunning: boolean): string | null {
  if (row.reverting) return 'Reverting…';
  if (locked) return 'This file is part of the running export. Revert when it ends.';
  if (recoveryRunning) return 'Checking for changes a crash interrupted. Revert when that ends.';
  if (row.state === 'analysing' || row.state === 'queued') return 'Being analysed. Revert when that ends.';
  return null;
}

/**
 * Puts the file's original back, in the background (about 10 s for a 6-minute track): the row shows
 * it, and everything else stays usable.
 */
export async function revert(fileId: number): Promise<void> {
  const lib = useLibrary.getState();
  const row = lib.rows[fileId];
  if (!row || revertBlock(row, isLocked(lib.locked, fileId), useRecovery.getState().running)) return;
  closeGridViewOf([fileId], 'being reverted');
  lib.setReverting(fileId);
  try {
    const result = await revertFile(fileId);
    useLibrary.getState().applyRevert(fileId, result);
    // The original could not be analysed again: what the row showed is gone.
    if (!result.update) pipeline().send({ type: 'ANALYSE', fileIds: [fileId] });
  } catch (err) {
    useLibrary.getState().revertFailed(fileId, ipcErrorOf(err));
  }
}

/** Writes a rekordbox XML of every analysed row; an exported copy is listed where it is. */
export async function writeXmlOfAnalysed(): Promise<void> {
  const lib = useLibrary.getState();
  const fileIds = xmlRowIds(lib);
  if (fileIds.length === 0 || lib.xmlWriting) return;
  lib.setXmlWriting(true);
  try {
    useLibrary.getState().applyXml(await writeRekordboxXml({ fileIds, out: null }));
  } catch (err) {
    useLibrary.getState().setXmlFailed(ipcErrorOf(err));
  }
}

/** Shows a file or folder in the Finder; a failure says so above the table. */
export function reveal(path: string): void {
  revealInFinder(path).catch((err: unknown) => {
    useLibrary.getState().setNotice(`Could not show it in the Finder: ${errorCopy(ipcErrorOf(err))}`);
  });
}

/** Asks for the folder copies go to; choosing none keeps the one set. */
export async function pickOutDir(): Promise<void> {
  const dir = await chooseExportFolder();
  if (dir) useSettings.getState().setOutDir(dir);
}
