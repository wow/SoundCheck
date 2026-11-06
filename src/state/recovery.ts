import { create } from 'zustand';
import {
  forgetPending,
  ipcErrorOf,
  recoveryStatus,
  type ForgetResult,
  type IpcError,
  type PendingChange,
  type RecoveryStatus,
} from '@/lib/ipc';

/** How often the state of the start-up recovery is read while it runs. */
const POLL_MS = 1000;

interface RecoveryState {
  /** The check for changes a crash interrupted is running: export, Revert and XML wait for it. */
  running: boolean;
  /** What it found, as the engine last said; `null` until it has said. */
  status: RecoveryStatus | null;
  /** The banner line about finished changes, or a failed check, was closed. */
  dismissed: boolean;
  /** The change being forgotten now, by journal id. */
  forgetting: string | null;
  /** Why the last Forget was refused, with the change it was for. */
  forgetError: { txn: string; error: IpcError } | null;
  /** What the last Forget kept, for the line that says where it is. */
  forgotten: ForgetResult | null;
  /** The change the Forget step is asking about, while it is open. */
  asking: PendingChange | null;
}

const INITIAL: RecoveryState = {
  running: false,
  status: null,
  dismissed: false,
  forgetting: null,
  forgetError: null,
  forgotten: null,
  asking: null,
};

export const useRecovery = create<RecoveryState>()(() => INITIAL);

/** Back to before the app asked (tests). */
export function resetRecovery(): void {
  useRecovery.setState(INITIAL, true);
}

function setStatus(status: RecoveryStatus): void {
  useRecovery.setState({ status, running: status.state === 'running' });
}

/**
 * How long to wait before reading again after a read failed, one per retry: a read can fail
 * while the app is still starting, so a few short waits before saying it could not be read.
 */
export const RETRY_MS = [250, 500, 1000] as const;

/** Follows the start-up recovery until it has ended. Returns a stop function. */
export function startRecoveryWatch(): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  const read = () => {
    recoveryStatus()
      .then((status) => {
        if (stopped) return;
        failures = 0;
        // Nothing known yet (an engine that does not answer this): as if it ended clean.
        if (!status) {
          useRecovery.setState({ running: false });
          return;
        }
        setStatus(status);
        if (status.state === 'running') timer = setTimeout(read, POLL_MS);
      })
      .catch((err: unknown) => {
        if (stopped) return;
        const wait = RETRY_MS[failures];
        failures += 1;
        if (wait !== undefined) {
          timer = setTimeout(read, wait);
          return;
        }
        // Not known: said so, and the engine still refuses what it must, with its own words.
        setStatus({
          state: 'failed',
          message: `what it found could not be read: ${ipcErrorOf(err).message}`,
        });
      });
  };
  read();
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}

/**
 * Gives up on a change recovery left pending: the engine ends it in its journal, deletes nothing,
 * and answers with the report without it, so its row goes.
 */
export async function forgetChange(txn: string): Promise<void> {
  if (useRecovery.getState().forgetting !== null) return;
  useRecovery.setState({ forgetting: txn, forgetError: null });
  try {
    const result = await forgetPending(txn);
    setStatus(result.status);
    useRecovery.setState({ forgotten: result });
  } catch (err) {
    const error = ipcErrorOf(err);
    // Refused, perhaps because it ended meanwhile: what the engine says now decides whether the
    // row stays, with why it was refused, or goes.
    const now = await recoveryStatus().catch(() => null);
    if (now) setStatus(now);
    const pending = useRecovery.getState().status;
    const still = pending?.state === 'finished' && pending.pending.some((p) => p.txn === txn);
    if (still || !now) useRecovery.setState({ forgetError: { txn, error } });
  } finally {
    useRecovery.setState({ forgetting: null });
  }
}

/** Opens the step that says what Forget means, for `change`; `null` closes it. */
export function askForget(change: PendingChange | null): void {
  useRecovery.setState({ asking: change });
}

/** Closes the line about finished changes or a failed check. */
export function dismissRecovery(): void {
  useRecovery.setState({ dismissed: true, forgotten: null });
}

/** Closes the line about what a Forget kept. */
export function dismissForgotten(): void {
  useRecovery.setState({ forgotten: null });
}
