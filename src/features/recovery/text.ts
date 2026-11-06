import { refusal, sentence, type ForgetResult, type IpcError, type PendingChange, type RecoveredChange } from '@/lib/ipc';

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
    todo: 'Restart SoundCheck to check again; if it fails again, make sure the SoundCheck Backups folder in your Music folder can be opened.',
  };
}

/**
 * What Forget does, from what the engine does: it ends the change in its journal so it is not
 * tried again, and deletes nothing. A change in place may have stopped before or after the new
 * version replaced the file; a copy into a folder, before its rename, is not there yet or is the
 * empty file that holds its name on an exFAT or FAT drive.
 */
export function forgetLines(change: PendingChange): string[] {
  const stops = 'SoundCheck stops trying to finish this change when it starts.';
  if (change.toFolder) {
    return [
      stops,
      'The copy is left as it is now: depending on how far the change got, there is no copy yet, an empty file holding its name, or the new copy. The original was not changed.',
      'Nothing is deleted: a half-written copy, if there is one, stays in that folder as a hidden file, and so does an empty file holding the copy\'s name. Both are listed once the change is forgotten.',
    ];
  }
  return [
    stops,
    'The file is left as it is now, which may be the original or the new version, depending on how far the change got.',
    'Nothing is deleted: a backup of the original, if the change made one, stays in the SoundCheck Backups folder, and a half-written copy, if there is one, stays next to the file as a hidden file.',
  ];
}

export interface KeptFile {
  path: string;
  /** What it is, and what to do with it when that matters. */
  what: string;
}

/** Whether `path` names one of the hidden temp files a change writes next to its target. */
function isTemp(path: string): boolean {
  const name = fileName(path);
  return name.startsWith('.') && name.includes('.soundcheck-tmp-');
}

/** What a Forget kept, once it is done: every file by its path, each said what it is. */
export function forgottenText(result: ForgetResult): { head: string; kept: KeptFile[] } {
  const head = `The change to ${fileName(result.path)} is forgotten. Nothing was deleted.`;
  const kept = result.kept.map((path): KeptFile => {
    if (path === result.backup) return { path, what: 'the backup of the original' };
    if (path === result.path) {
      return {
        path,
        what: 'may be an empty file holding the copy\'s name; remove it before exporting to that folder again, or that export skips the name',
      };
    }
    if (isTemp(path)) {
      return {
        path,
        what: 'a hidden copy the change was writing, which may be complete; the Finder shows hidden files with Cmd+Shift+.',
      };
    }
    return { path, what: 'a file the change made' };
  });
  return { head, kept };
}

/**
 * A Forget the engine refused, for a change still pending after the status was read again: the
 * engine's words without the class name it starts with ("invalid argument: "), then what to do.
 */
export function forgetRefusedText(error: IpcError): string {
  const why = sentence(error.message.trim().replace(/^invalid argument:\s*/i, '')).replace(/\.$/, '');
  const todo =
    error.kind === 'invalidArgument'
      ? 'Try again once it has stopped; if it is still refused, restart SoundCheck to check it again.'
      : refusal(error, 'banner').todo;
  return `${why}. ${todo}`;
}
