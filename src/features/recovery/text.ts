import { sentence, type ForgetResult, type RecoveredChange } from '@/lib/ipc';

/**
 * The words of the start-up recovery: what a crash interrupted, what became of it, and what
 * Forget does. A change is one file SoundCheck was writing (an export or a revert) when the app
 * stopped; at start it is finished, undone, or left for later when its drive is not there.
 */

/** The file's name, the last part of its path. */
export function fileName(path: string): string {
  const parts = path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function changes(n: number): string {
  return n === 1 ? '1 change' : `${n} changes`;
}

export const RUNNING_TEXT = 'Finishing changes from the last session…';
export const RUNNING_DETAIL = 'Export, Revert and the rekordbox XML wait until it is done.';

/** "2 changes could not be finished", the head of the pending list. */
export function pendingHead(n: number): string {
  return `${changes(n)} from the last session could not be finished`;
}

export const PENDING_TODO =
  'SoundCheck tries again at every start: connect the drive the file is on and restart, or forget a change you no longer need.';

/** Why recovery left a change, in words: the engine's, but for its two one-word answers. */
export function pendingReason(reason: string): string {
  if (reason === 'running') return 'It is still running in another SoundCheck window';
  if (reason === 'cleanup incomplete') return 'Its temporary files could not all be removed';
  return sentence(reason);
}

/** "2 changes from the last session were finished", and what an undone one means for its file. */
export function recoveredText(recovered: readonly RecoveredChange[]): string | null {
  const n = recovered.length;
  if (n === 0) return null;
  const undone = recovered.filter((r) => r.outcome === 'rolledBack').length;
  const finished = n - undone;
  if (undone === 0)
    return `${changes(n)} from the last session ${n === 1 ? 'was' : 'were'} finished.`;
  if (finished === 0) {
    return n === 1
      ? '1 change from the last session was undone; its file is as it was.'
      : `${n} changes from the last session were undone; their files are as they were.`;
  }
  return `${changes(n)} from the last session were settled: ${finished} finished, ${undone} undone (${
    undone === 1 ? 'its file is' : 'their files are'
  } as ${undone === 1 ? 'it was' : 'they were'}).`;
}

/** The failed check, what it means, and what to do. */
export function failedText(message: string): { what: string; why: string; todo: string } {
  return {
    what: 'The changes from the last session could not be checked',
    why: sentence(message.trim()).replace(/\.$/, ''),
    todo: 'Nothing was changed. Restart SoundCheck to check again; if it fails again, make sure the SoundCheck Backups folder in your Music folder can be opened.',
  };
}

/**
 * What Forget does, from what the engine does: it ends the change in its journal so it is not
 * tried again, and deletes nothing.
 */
export const FORGET_LINES = [
  'SoundCheck stops trying to finish this change when it starts.',
  'The file is left as it is now, which may be the version before or after the change, depending on how far the change got.',
  'Nothing is deleted: a backup of the original, if the change made one, stays in the SoundCheck Backups folder, and a half-written copy, if there is one, stays next to the file as a hidden file.',
];

/** What a Forget kept, once it is done. */
export function forgottenText(result: ForgetResult): string {
  const name = fileName(result.path);
  const backup = result.backup ? ` The backup of the original is at ${result.backup}.` : '';
  const others = result.kept.filter((p) => p !== result.backup).length;
  const kept =
    others > 0
      ? ` ${others === 1 ? '1 other file it made stays' : `${others} other files it made stay`} too.`
      : '';
  return `The change to ${name} is forgotten. Nothing was deleted.${backup}${kept}`;
}
