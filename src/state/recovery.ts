import { create } from 'zustand';
import { recoveryStatus } from '@/lib/ipc';

/** How often the state of the start-up recovery is read while it runs. */
const POLL_MS = 1000;

interface RecoveryState {
  /** The check for changes a crash interrupted is running: export, Revert and XML wait for it. */
  running: boolean;
}

export const useRecovery = create<RecoveryState>()(() => ({ running: false }));

/** Follows the start-up recovery until it has ended. Returns a stop function. */
export function startRecoveryWatch(): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const read = () => {
    recoveryStatus()
      .then((status) => {
        if (stopped) return;
        const running = status?.state === 'running';
        useRecovery.setState({ running });
        if (running) timer = setTimeout(read, POLL_MS);
      })
      .catch(() => {
        // Not known: the engine still refuses what it must, with its own words.
        if (!stopped) useRecovery.setState({ running: false });
      });
  };
  read();
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}
