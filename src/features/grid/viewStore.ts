import { create } from 'zustand';
import type { View } from './geometry';

/**
 * What the grid view shows and where the pointer is: kept apart from the track store so that
 * scrolling and hovering redraw the canvas without re-rendering the rail.
 */
export interface ViewState {
  view: View;
  /** The sample under the pointer, or null. */
  hover: number | null;
  /** Show the analysed grid, dashed, under the edited one (`A`). */
  ghost: boolean;
  setView(view: View): void;
  setHover(sample: number | null): void;
  toggleGhost(): void;
}

export const useView = create<ViewState>()((set) => ({
  view: { start: 0, samplesPerPx: 500, widthPx: 1 },
  hover: null,
  ghost: false,
  setView: (view) => set({ view }),
  setHover: (hover) => set({ hover }),
  toggleGhost: () => set((s) => ({ ghost: !s.ghost })),
}));
