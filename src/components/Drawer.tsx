import { useRef, type ReactNode } from 'react';
import { useDialogFocus } from './useDialogFocus';

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
  const onKeyDown = useDialogFocus(panel, { onClose, returnFocus, focus });

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
