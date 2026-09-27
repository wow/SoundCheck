import type { MouseEvent } from 'react';

/**
 * Who a key belongs to: the window's shortcuts (Space, Enter, letters) or the focused element.
 * A text field takes every key; a button, link or menu item takes Enter and Space.
 */
export function ownsKey(target: EventTarget | null, key: string): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLSelectElement ||
    target instanceof HTMLTextAreaElement
  ) {
    return true;
  }
  return (key === 'Enter' || key === ' ') && target.closest(CONTROL) !== null;
}

const CONTROL = 'button, a[href], [role="button"], [role="menuitem"]';

/**
 * For `onMouseDownCapture` at the root: a click on a button does not move focus to it, as in a
 * native Mac app, so Space and Enter stay the window's shortcuts after a click. Tab still
 * focuses buttons for keyboard use; text fields still take focus.
 */
export function keepFocusOnClick(e: MouseEvent): void {
  const target = e.target;
  if (
    target instanceof Element &&
    target.closest(CONTROL) !== null &&
    target.closest('input, textarea, select') === null
  ) {
    e.preventDefault();
  }
}
