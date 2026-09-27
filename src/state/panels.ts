import { create } from 'zustand';

/**
 * What is folded away in a window too narrow for everything: the settings rail, shown as a
 * drawer when open, and the name filter, shown as an icon until opened.
 */
interface PanelsState {
  settings: boolean;
  search: boolean;
  setSettings(open: boolean): void;
  setSearch(open: boolean): void;
}

export const usePanels = create<PanelsState>()((set) => ({
  settings: false,
  search: false,
  setSettings(open) {
    set({ settings: open });
  },
  setSearch(open) {
    set({ search: open });
  },
}));
