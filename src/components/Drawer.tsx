import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * A rail shown over the right side of the window when it does not fit beside the content (a
 * window narrower than 1000 px). Focus moves in when it opens, stays inside while it is open,
 * and goes back where it was when it closes, or to `returnFocus()` when that is gone (the
 * button that opened it disappears when the window widens); `Esc` or a press outside closes it.
 * It slides in over 150 ms, not at all under reduced motion.
 */
export function Drawer({
  label,
  onClose,
  returnFocus,
  children,
}: {
  label: string;
  onClose: () => void;
  returnFocus?: () => HTMLElement | null;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const fallback = useRef(returnFocus);
  useEffect(() => {
    fallback.current = returnFocus;
  });
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const first = panel.current?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? panel.current)?.focus();
    return () => {
      // Nothing focused before (the page itself) counts as gone too.
      const kept = before && before !== document.body && before.isConnected;
      (kept ? before : (fallback.current?.() ?? null))?.focus();
    };
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
