import { useRef, useState, type ReactNode } from 'react';
import { useDialogFocus } from './useDialogFocus';

/** A press outside this soon after opening is the end of a double click on what opened it. */
export const BACKDROP_GRACE_MS = 300;

/**
 * A panel over the window that asks for one decision: keeps focus inside while open and gives it
 * back when it closes; `Esc` or a press outside closes it. Never taller than the window: its
 * children scroll inside.
 */
export function Modal({
  label,
  onClose,
  children,
}: {
  label: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const [opened] = useState(() => performance.now());
  const onKeyDown = useDialogFocus(panel, { onClose, focus: 'panel' });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-[rgba(8,10,13,.55)]"
        onMouseDown={() => {
          if (performance.now() - opened >= BACKDROP_GRACE_MS) onClose();
        }}
      />
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        className="relative flex max-h-full w-[520px] max-w-full flex-col rounded-[10px] border border-line bg-bg-1 shadow-[0_12px_40px_rgba(0,0,0,.5)] outline-none"
      >
        {children}
      </div>
    </div>
  );
}
