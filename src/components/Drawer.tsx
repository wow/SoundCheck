import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * A rail shown over the right side of the window when it does not fit beside the content (a
 * window narrower than 1000 px). Focus moves in when it opens (to its first control, or with
 * `focus="panel"` to the panel itself, where Space cannot press a button by habit), stays inside
 * while it is open, even when the focused control goes away, and goes back where it was when it
 * closes, or to `returnFocus()` when that is gone (the button that opened it disappears when
 * the window widens). `Esc`, wherever focus is, or a press outside closes it. It slides in over
 * 150 ms, not at all under reduced motion.
 */
export function Drawer({
  label,
  onClose,
  returnFocus,
  focus = 'first',
  children,
}: {
  label: string;
  onClose: () => void;
  returnFocus?: () => HTMLElement | null;
  focus?: 'first' | 'panel';
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
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

  const onKeyDown = (e: KeyboardEvent) => {
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

  return (
    <div className="fixed inset-0 z-40">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-[rgba(8,10,13,.55)]"
        onMouseDown={onClose}
      />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        className="absolute inset-y-0 right-0 flex w-[300px] max-w-[85vw] flex-col overflow-y-auto border-l border-line bg-bg-1 shadow-[-12px_0_32px_rgba(0,0,0,.45)] outline-none motion-safe:animate-[drawer-in_150ms_ease-out]"
      >
        {children}
      </div>
    </div>
  );
}
