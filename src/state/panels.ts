import { create } from 'zustand';

/** Which rail is open as a drawer, in a window too narrow to show it beside the content. */
interface PanelsState {
  settings: boolean;
  setSettings(open: boolean): void;
}

export const usePanels = create<PanelsState>()((set) => ({
  settings: false,
  setSettings(open) {
    set({ settings: open });
  },
}));
