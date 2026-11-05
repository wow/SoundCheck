import type { IpcError } from './generated/IpcError';
import type { IpcErrorKind } from './generated/IpcErrorKind';

/**
 * What to do about a refused or failed command, for every error class the engine sends and every
 * place it can be shown. The engine words what happened and why (`IpcError.message`); this adds
 * the step that gets the user going again, worded for where it is read:
 *
 * - `row`: a file's own row (its Action cell and the details under the table), after an export
 *   or a revert of that one file;
 * - `confirm`: the confirm step, before anything is written, for a file or the whole plan;
 * - `banner`: the line above the table, for a job that could not start, the rekordbox XML, a
 *   setting the engine refused, or the start-up recovery.
 */
export type Surface = 'row' | 'confirm' | 'banner';

export const SURFACES: readonly Surface[] = ['row', 'confirm', 'banner'];

type Todo = Partial<Record<Surface, string>>;

const RETRY = 'Try again; if it keeps happening, restart SoundCheck.';
const RECONVERT = 'Convert it to AIFF, WAV or FLAC in another app, then add that file.';

/**
 * The what-to-do line of each class, per surface; a surface missing from a class is one it never
 * reaches. One `switch`, so a class added on the Rust side fails to compile here until it has
 * its words.
 */
export function todoFor(kind: IpcErrorKind): Todo {
  switch (kind) {
    case 'unsupportedFormat':
      return { row: RECONVERT, confirm: RECONVERT, banner: RECONVERT };
    case 'unsupportedChannels': {
      const stereo = 'Bounce a stereo or mono version, then add that file.';
      return { row: stereo, confirm: stereo, banner: stereo };
    }
    case 'corrupt': {
      const fresh =
        'Get a fresh copy of the file (download or bounce it again), then add that one.';
      return {
        row: fresh,
        confirm: fresh,
        banner: 'Move that file away, or choose another folder.',
      };
    }
    case 'io':
      return {
        row: 'Check that the drive is connected and the file opens, then try again.',
        confirm: 'Check that the drive is connected, then press Export again.',
        banner:
          'Check that the drive is connected and the folder can be written to, then try again.',
      };
    case 'cancelled':
      return {
        row: 'Export it again when you are ready.',
        confirm: 'Press Export again when you are ready.',
        banner: 'Start it again when you are ready.',
      };
    case 'notDjSafe': {
      const safe = 'Bounce it at 44.1 or 48 kHz, 16 or 24 bit, then add that file.';
      return { row: safe, confirm: safe, banner: safe };
    }
    case 'drmProtected': {
      const free = 'Use a copy without DRM, such as the download from the store.';
      return { row: free, confirm: free, banner: free };
    }
    case 'wouldClip':
      return {
        row: 'Lower the target or raise the ceiling, then export it again.',
        confirm: 'Lower the target or raise the ceiling, then press Export again.',
        banner: 'Lower the target or raise the ceiling, then try again.',
      };
    case 'modelUnavailable': {
      const reinstall = 'Install SoundCheck again; the beat model comes with it.';
      return { row: reinstall, confirm: reinstall, banner: reinstall };
    }
    case 'invalidArgument':
      return { row: RETRY, confirm: 'Close this and press Export again.', banner: RETRY };
    case 'internal':
      return { row: RETRY, confirm: RETRY, banner: RETRY };
    case 'rekordboxUsbExport': {
      const library = 'Process the library copy, then export the USB again from rekordbox.';
      return { row: library, confirm: library, banner: library };
    }
    case 'inPlaceRefused':
      return {
        row: 'Fix that and export it again, or export it To folder.',
        confirm: 'Fix that and press Export again, or choose To folder.',
      };
    case 'noSpace': {
      const room = 'Free some space on that drive, or choose another folder.';
      return { row: room, confirm: room, banner: room };
    }
    case 'verifyFailed':
      return {
        row: 'Export it again; if it fails again, check the drive in Disk Utility.',
        confirm: 'Press Export again; if it fails again, check the drive in Disk Utility.',
        banner: 'Try again; if it fails again, check the drive in Disk Utility.',
      };
    case 'fileChanged':
      return {
        row: 'Press Analyse to read it again.',
        confirm: 'Press Analyse for it, then press Export again.',
        banner: 'Press Analyse for the files that changed, then try again.',
      };
    case 'nothingToUndo': {
      const none = 'Nothing to do: the file is as SoundCheck found it.';
      return { row: none, banner: none };
    }
    case 'alreadyExists': {
      const other = 'Rename or move that file, or choose another folder.';
      return { row: other, confirm: other, banner: other };
    }
    case 'listedTwice': {
      const once = 'Nothing to do: it is exported once.';
      return { row: once, confirm: once };
    }
    case 'sameOutputName': {
      const rename = 'Rename one of them, or choose another folder.';
      return { row: rename, confirm: rename };
    }
    case 'recoveryRunning':
      return {
        row: 'Nothing was changed. Try again once the line above the table has gone.',
        confirm: 'Nothing was changed. Press Export again once the line above the table has gone.',
        banner: 'Nothing was changed. Try again in a moment.',
      };
    case 'busy':
      return {
        row: 'Wait for it to finish, then try again.',
        confirm: 'Wait for it to finish, then press Export again.',
        banner: 'Wait for it to finish, then try again.',
      };
    case 'settingsChanged': {
      const again = 'Nothing was written. Press Export again to see the new numbers.';
      return { confirm: again, banner: again, row: again };
    }
    default: {
      const unknown: never = kind;
      return unknown;
    }
  }
}

/** The surfaces a class can be shown on: those it has words for. */
export function surfacesOf(kind: IpcErrorKind): Surface[] {
  const todo = todoFor(kind);
  return SURFACES.filter((s) => todo[s] !== undefined);
}

/**
 * The engine's own advice at the end of its message, for the classes that word one: a refusal in
 * place says what to change about the file ("Unlock it, or write a copy to a folder"), which is
 * more exact than any line written here. It is the last sentence and never names a path.
 */
const ADVISED: ReadonlySet<IpcErrorKind> = new Set([
  'inPlaceRefused',
  'rekordboxUsbExport',
  'noSpace',
]);
const ADVICE = /\.\s+([A-Z][^/]*)$/;

/** Endings the engine adds that a line here says better, per class. */
const SAID_HERE: Partial<Record<IpcErrorKind, RegExp>> = {
  busy: /;\s*try again when that has finished\.?$/,
  recoveryRunning: /;\s*nothing was done\.?$/,
  settingsChanged: /;\s*nothing was done\.?$/,
};

/** Text from the engine, which words its reasons to follow a colon, as a sentence of its own. */
export function sentence(text: string): string {
  return text.charAt(0).toLocaleUpperCase() + text.slice(1);
}

function stop(text: string): string {
  const t = text.trim();
  return /[.!?…]$/.test(t) ? t : `${t}.`;
}

export interface Refusal {
  /** Why, in the engine's words; a file's own path reads "This file" on its row. */
  why: string;
  /** What to do, for this surface. */
  todo: string;
}

/** The why and the what-to-do of `error` on `surface`; `path`: the row's file. */
export function refusal(error: IpcError, surface: Surface, path?: string): Refusal {
  let why = error.message.trim();
  let advice: string | null = null;
  const said = SAID_HERE[error.kind];
  if (said) why = why.replace(said, '');
  if (ADVISED.has(error.kind)) {
    const m = ADVICE.exec(why);
    if (m?.[1]) {
      advice = stop(m[1]);
      why = why.slice(0, m.index + 1);
    }
  }
  if (path && why.startsWith(path)) why = `This file${why.slice(path.length)}`;
  const todo = todoFor(error.kind);
  // A surface a class is not expected on still gets a way forward, never an empty line.
  const line = advice ?? todo[surface] ?? todo.banner ?? todo.confirm ?? todo.row ?? RETRY;
  return { why: why ? sentence(why).replace(/\.$/, '') : 'The command failed', todo: line };
}
