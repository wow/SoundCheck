import { playerVolume } from '@/lib/ipc';
import { loadMonitor, saveMonitor } from '@/lib/persist';
import { DEFAULT_MONITOR, heardDb, useMonitor } from '@/state/monitor';

/** Wait after the last change before the volume is saved, so a drag writes the file once. */
export const MONITOR_SAVE_DELAY_MS = 300;

/**
 * Reads the saved monitor volume and tells the player, then keeps the player and the saved copy
 * in step with the store. While a volume command is on its way, only the newest value waits
 * behind it, so a drag never queues up a backlog. Returns a stop function.
 */
export function startMonitorSync(): () => void {
  let sent: number | null | undefined;
  let inFlight = false;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let lastSaved = '';

  const send = () => {
    const m = useMonitor.getState();
    if (!m.hydrated || inFlight) return;
    const db = heardDb(m);
    if (db === sent) return;
    inFlight = true;
    sent = db;
    void playerVolume(db).then(
      () => {
        inFlight = false;
        // A change made meanwhile goes now.
        send();
      },
      () => {
        // Not tried again until the next change, so a player that keeps refusing is not
        // asked in a loop; the slider keeps showing the user's choice.
        inFlight = false;
        sent = undefined;
        // A change made meanwhile is new, not a retry: it goes now.
        if (heardDb(useMonitor.getState()) !== db) send();
      },
    );
  };

  const save = () => {
    const { db, muted, hydrated } = useMonitor.getState();
    if (!hydrated) return;
    const key = JSON.stringify({ db, muted });
    if (key === lastSaved) return;
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      lastSaved = key;
      void saveMonitor({ db, muted });
    }, MONITOR_SAVE_DELAY_MS);
  };

  const unsubscribe = useMonitor.subscribe((m, prev) => {
    if (m.db !== prev.db || m.muted !== prev.muted || m.hydrated !== prev.hydrated) send();
    save();
  });
  void loadMonitor().then((saved) => {
    // What was read needs no writing back.
    const merged = { ...DEFAULT_MONITOR, ...saved };
    lastSaved = JSON.stringify({ db: merged.db, muted: merged.muted });
    useMonitor.getState().hydrate(saved);
  });
  return () => {
    unsubscribe();
    if (saveTimer !== null) clearTimeout(saveTimer);
  };
}
