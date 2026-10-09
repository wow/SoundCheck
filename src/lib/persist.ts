import { load, type Store } from '@tauri-apps/plugin-store';
import { MONITOR_SCHEMA, sanitizeMonitor, type SavedMonitor } from '@/state/monitor';
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

/** The monitor volume sits beside the settings, under its own key: it never replans a row. */
const MONITOR_KEY = 'monitor';

/** The saved monitor volume, or none when there is none yet or the store cannot be read. */
export async function loadMonitor(): Promise<Partial<SavedMonitor>> {
  try {
    const saved = await (await open()).get<Record<string, unknown>>(MONITOR_KEY);
    if (!saved || saved.schema !== MONITOR_SCHEMA) return {};
    return sanitizeMonitor(saved);
  } catch {
    return {};
  }
}

/** Saves the monitor volume; a failure only means it is not remembered. */
export async function saveMonitor(monitor: SavedMonitor): Promise<void> {
  try {
    await (await open()).set(MONITOR_KEY, { schema: MONITOR_SCHEMA, ...monitor });
  } catch {
    // Not remembered this time.
  }
}

/** The track list lives in its own file, so it can grow without touching the settings. */
const LIST_FILE = 'library.json';
const LIST_KEY = 'tracks';
const LIST_SCHEMA = 1;

let listStore: Promise<Store> | null = null;

function openList(): Promise<Store> {
  listStore ??= load(LIST_FILE, { autoSave: 500 });
  return listStore;
}

/** The file paths of the last session's track list, in order; none when there is none. */
export async function loadTrackList(): Promise<string[]> {
  try {
    const saved = await (await openList()).get<{ schema?: unknown; paths?: unknown }>(LIST_KEY);
    if (!saved || saved.schema !== LIST_SCHEMA || !Array.isArray(saved.paths)) return [];
    return saved.paths.filter((p): p is string => typeof p === 'string');
  } catch {
    return [];
  }
}

/** Saves the track list's file paths; a failure only means the list is not remembered. */
export async function saveTrackList(paths: string[]): Promise<void> {
  try {
    await (await openList()).set(LIST_KEY, { schema: LIST_SCHEMA, paths });
  } catch {
    // Not remembered this time.
  }
}
