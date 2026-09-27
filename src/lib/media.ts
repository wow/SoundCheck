import { useSyncExternalStore } from 'react';

/** Windows this narrow show the rails as drawers. */
export const NARROW = '(max-width: 999px)';

/** Whether `query` matches now, following the window as it is resized. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (changed) => {
      const list = typeof window.matchMedia === 'function' ? window.matchMedia(query) : null;
      list?.addEventListener('change', changed);
      return () => list?.removeEventListener('change', changed);
    },
    () => (typeof window.matchMedia === 'function' ? window.matchMedia(query).matches : false),
    () => false,
  );
}
