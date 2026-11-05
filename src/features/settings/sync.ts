import { errorCopy, ipcErrorOf, setDecideSettings } from '@/lib/ipc';
import { loadSettings, saveSettings } from '@/lib/persist';
import { useLibrary } from '@/state/library';
import { decideSettings, savedPart, useSettings } from '@/state/settings';

/** The latest change of the decide settings on its way to the engine. */
let landing: Promise<unknown> = Promise.resolve();

/** Resolves once every change made so far has reached the engine (or been refused by it). */
export function settingsLanded(): Promise<unknown> {
  return landing;
}

/**
 * Reads the saved settings, then keeps the engine's decide settings and the saved copy in step
 * with the store: every change is saved, and one that decides differently replans every analysed
 * row (the export settings do not). Returns a stop function.
 */
export function startSettingsSync(): () => void {
  let lastSaved = '';
  let lastDecide = '';
  const push = () => {
    const s = useSettings.getState();
    if (!s.hydrated) return;
    const saved = JSON.stringify(savedPart(s));
    if (saved === lastSaved) return;
    lastSaved = saved;
    void saveSettings(savedPart(s));
    const decide = JSON.stringify(decideSettings(s));
    if (decide === lastDecide) return;
    lastDecide = decide;
    const sent = setDecideSettings(decideSettings(s))
      .then((replan) => useLibrary.getState().applyReplan(replan))
      .catch((err: unknown) => {
        useLibrary.getState().setNotice(`The settings could not be applied: ${errorCopy(ipcErrorOf(err))}`);
      });
    landing = Promise.all([landing, sent]);
  };
  const unsubscribe = useSettings.subscribe(push);
  void loadSettings().then((saved) => useSettings.getState().hydrate(saved));
  return unsubscribe;
}
