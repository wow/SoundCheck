import { load, type Store } from '@tauri-apps/plugin-store';
import { SETTINGS_SCHEMA, sanitize, type SavedSettings } from '@/state/settings';

/** Settings live in the app's own store file; nothing is written next to the music. */
const FILE = 'settings.json';
const KEY = 'settings';

let store: Promise<Store> | null = null;

function open(): Promise<Store> {
  store ??= load(FILE, { autoSave: 200 });
  return store;
}

/** The saved settings, or none when there are none yet or the store cannot be read. */
export async function loadSettings(): Promise<Partial<SavedSettings>> {
  try {
    const saved = await (await open()).get<Record<string, unknown>>(KEY);
    if (!saved || saved.schema !== SETTINGS_SCHEMA) return {};
    return sanitize(saved);
  } catch {
    return {};
  }
}

/** Saves the settings; a failure only means they are not remembered. */
export async function saveSettings(settings: SavedSettings): Promise<void> {
  try {
    await (await open()).set(KEY, { schema: SETTINGS_SCHEMA, ...settings });
  } catch {
    // Not remembered this time; the app keeps working with the values on screen.
  }
}
