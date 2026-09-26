import { setDecideSettings } from '@/lib/ipc';
import { loadSettings, saveSettings } from '@/lib/persist';
import { useLibrary } from '@/state/library';
import { decideSettings, savedPart, useSettings } from '@/state/settings';

/**
 * Reads the saved settings, then keeps the engine's decide settings and the saved copy in step
 * with the store: every change replans every analysed row and is saved. Returns a stop function.
 */
export function startSettingsSync(): () => void {
  let last = '';
  const push = () => {
    const s = useSettings.getState();
    if (!s.hydrated) return;
    const key = JSON.stringify(savedPart(s));
    if (key === last) return;
    last = key;
    void saveSettings(savedPart(s));
    void setDecideSettings(decideSettings(s))
      .then((replan) => useLibrary.getState().applyReplan(replan))
      .catch((err: unknown) => {
        const message =
          typeof err === 'object' && err !== null && 'message' in err
            ? String((err as { message: unknown }).message)
            : String(err);
        useLibrary.getState().setNotice(`The settings could not be applied: ${message}`);
      });
  };
  const unsubscribe = useSettings.subscribe(push);
  void loadSettings().then((saved) => useSettings.getState().hydrate(saved));
  return unsubscribe;
}
