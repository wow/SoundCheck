import { create } from 'zustand';

/**
 * What is folded away in a window too narrow for everything: the settings rail and the grid
 * view's details rail, shown as drawers when open, and the name filter, shown as an icon until
 * opened.
 */
interface PanelsState {
  settings: boolean;
  details: boolean;
  search: boolean;
  setSettings(open: boolean): void;
  setDetails(open: boolean): void;
  setSearch(open: boolean): void;
}

export const usePanels = create<PanelsState>()((set) => ({
  settings: false,
  details: false,
  search: false,
  setSettings(open) {
    set({ settings: open });
  },
  setDetails(open) {
    set({ details: open });
  },
  setSearch(open) {
    set({ search: open });
  },
}));
