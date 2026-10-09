import { useSyncExternalStore } from 'react';

/** Windows this narrow show the rails as drawers. */
export const NARROW = '(max-width: 999px)';

/**
 * Calls `changed` whenever the device pixel ratio changes (the window moved to another display,
 * or the page zoomed), which no resize observer reports. Returns a stop function.
 */
export function watchPixelRatio(changed: () => void): () => void {
  let list: MediaQueryList | null = null;
  let armed = 0;
  const onChange = () => {
    // The query is for one ratio: once it no longer holds, watch the new one.
    if ((window.devicePixelRatio || 1) !== armed) arm();
    changed();
  };
  function arm() {
    list?.removeEventListener?.('change', onChange);
    list = null;
    if (typeof window.matchMedia !== 'function') return;
    armed = window.devicePixelRatio || 1;
    list = window.matchMedia(`(resolution: ${armed}dppx)`);
    list.addEventListener?.('change', onChange);
  }
  arm();
  return () => list?.removeEventListener?.('change', onChange);
}

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
