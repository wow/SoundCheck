import type { IpcError } from './generated/IpcError';
import { refusal, type Surface } from './refusalCopy';

export { refusal, sentence, SURFACES, surfacesOf, todoFor } from './refusalCopy';
export type { Refusal, Surface } from './refusalCopy';

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
  const message =
    err instanceof Error ? err.message : typeof err === 'string' ? err : 'The command failed.';
  return { kind: 'internal', message, fileId: null };
}

/** Why, then what to do, as one text: for a line with room for both (`banner` unless said). */
export function errorCopy(error: IpcError, surface: Surface = 'banner'): string {
  const { why, todo } = refusal(error, surface);
  return `${why}. ${todo}`;
}

/** {@link errorCopy} on a file's own row, where its long path would push out what it says. */
export function rowErrorCopy(error: IpcError, path: string, surface: Surface = 'row'): string {
  const { why, todo } = refusal(error, surface, path);
  return `${why}. ${todo}`;
}
