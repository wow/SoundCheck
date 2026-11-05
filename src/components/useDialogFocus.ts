import { useEffect, useRef, type KeyboardEvent, type RefObject } from 'react';

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Focus for a modal panel: it moves in when the panel opens (to its first control, or with
 * `focus="panel"` to the panel itself, where Space cannot press a button by habit), stays inside
 * while it is open, even when the focused control goes away, and goes back where it was when it
 * closes, or to `returnFocus()` when that is gone. `Esc`, wherever focus is, closes it. Returns
 * the panel's key handler.
 */
export function useDialogFocus(
  panel: RefObject<HTMLDivElement | null>,
  {
    onClose,
    returnFocus,
    focus = 'first',
  }: {
    onClose: () => void;
    returnFocus?: () => HTMLElement | null;
    focus?: 'first' | 'panel';
  },
): (e: KeyboardEvent) => void {
  const latest = useRef({ returnFocus, onClose });
  useEffect(() => {
    latest.current = { returnFocus, onClose };
  });
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const first = focus === 'first' ? panel.current?.querySelector<HTMLElement>(FOCUSABLE) : null;
    (first ?? panel.current)?.focus();
    // Keys and focus outside the panel while it is open: after a focused control inside goes
    // away (a card removed, a button relabelled) focus lands on the page, not in here.
    const onDocumentKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Before the window's own Esc (back, or clearing the filter) can see it.
        e.stopPropagation();
        latest.current.onClose();
      } else if (e.key === 'Tab' && !panel.current?.contains(document.activeElement)) {
        e.preventDefault();
        (panel.current?.querySelector<HTMLElement>(FOCUSABLE) ?? panel.current)?.focus();
      }
    };
    const onFocusIn = (e: FocusEvent) => {
      if (e.target instanceof Node && !panel.current?.contains(e.target)) panel.current?.focus();
    };
    document.addEventListener('keydown', onDocumentKey);
    document.addEventListener('focusin', onFocusIn);
    return () => {
      document.removeEventListener('keydown', onDocumentKey);
      document.removeEventListener('focusin', onFocusIn);
      // Nothing focused before (the page itself) counts as gone too.
      const kept = before && before !== document.body && before.isConnected;
      (kept ? before : (latest.current.returnFocus?.() ?? null))?.focus();
    };
    // Set up once per opening; `focus` only matters then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      // Not also the window's Esc (which clears the table's filter).
      e.stopPropagation();
      onClose();
    } else if (e.key === 'Tab') {
      const items = [...(panel.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
      const first = items[0];
      const last = items[items.length - 1];
      const at = items.indexOf(document.activeElement as HTMLElement);
      if (!first || !last) {
        e.preventDefault();
      } else if (at < 0) {
        // Focus on the panel itself (a click on its background): into its controls.
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && at === 0) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && at === items.length - 1) {
        e.preventDefault();
        first.focus();
      }
    }
  };
}
