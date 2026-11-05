import type { IpcError } from './generated/IpcError';

function isIpcError(err: unknown): err is IpcError {
  return (
    typeof err === 'object' &&
    err !== null &&
    typeof (err as IpcError).kind === 'string' &&
    typeof (err as IpcError).message === 'string'
  );
}

/** What a rejected command rejected with, as the typed error every Rust command sends. */
export function ipcErrorOf(err: unknown): IpcError {
  if (isIpcError(err)) return { kind: err.kind, message: err.message, fileId: err.fileId ?? null };
  const message = err instanceof Error ? err.message : typeof err === 'string' ? err : 'The command failed.';
  return { kind: 'internal', message, fileId: null };
}

/** Text from the engine, which words its reasons to follow a colon, as a sentence of its own. */
export function sentence(text: string): string {
  return text.charAt(0).toLocaleUpperCase() + text.slice(1);
}

/** The refusals that are about timing rather than the file, in words the user can act on. */
export function errorCopy(error: IpcError): string {
  switch (error.kind) {
    case 'busy':
      return 'Another job is working on one of these files. Wait for it to finish, then try again.';
    case 'recoveryRunning':
      return 'SoundCheck is still checking for changes a crash interrupted. Nothing was changed; try again in a moment.';
    case 'settingsChanged':
      return 'The loudness settings changed after this summary, so nothing was written. Press Export again to see the new numbers.';
    default:
      return sentence(error.message);
  }
}

/** {@link errorCopy} on a file's own row, where its long path would push out what it says. */
export function rowErrorCopy(error: IpcError, path: string): string {
  const copy = errorCopy(error);
  return copy.startsWith(path) ? `This file${copy.slice(path.length)}` : copy;
}
