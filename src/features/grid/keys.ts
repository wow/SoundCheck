/**
 * The grid view's keys, as actions. Nudge and beat-1 keys go by physical key (`code`), so they
 * work on any keyboard layout; letters, and Cmd+Z, go by the character typed, and on a layout
 * that types no Latin letters (Cyrillic, Greek) by the key's place.
 */

export type GridKey =
  | { type: 'confirmNext' }
  | { type: 'nextReview' }
  | { type: 'back' }
  | { type: 'playPause' }
  | { type: 'toBarOne' }
  | { type: 'click' }
  | { type: 'ghost' }
  | { type: 'barOneHere'; free: boolean }
  | { type: 'nudge'; direction: 1 | -1; unit: 'ms' | '10ms' | 'beat' }
  | { type: 'beatOne'; beat: number }
  | { type: 'tap' }
  | { type: 'meter' }
  | { type: 'details' }
  | { type: 'reset' }
  | { type: 'undo' }
  | { type: 'redo' }
  | { type: 'pan'; bars: number }
  | { type: 'zoom'; factor: number }
  | { type: 'version' }
  | { type: 'match' }
  | { type: 'volume'; step: 1 | -1 };

export interface KeyLike {
  key: string;
  code: string;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}

export function gridKey(e: KeyLike): GridKey | null {
  const command = e.metaKey || e.ctrlKey;
  if (command) {
    // Undo goes by the letter typed, as macOS does: the key labelled Z is `KeyY` on a German
    // keyboard and `KeyW` on a French one. A non-Latin layout falls back to the key's place.
    if (letterOf(e) === 'z') return e.shiftKey ? { type: 'redo' } : { type: 'undo' };
    return null;
  }
  if (e.code === 'Comma' || e.code === 'Period') {
    return {
      type: 'nudge',
      direction: e.code === 'Comma' ? -1 : 1,
      unit: e.altKey ? 'beat' : e.shiftKey ? '10ms' : 'ms',
    };
  }
  const digit = /^Digit([1-9])$/.exec(e.code);
  if (digit && !e.shiftKey && !e.altKey) return { type: 'beatOne', beat: Number(digit[1]) };
  switch (e.key) {
    case 'Enter':
      return { type: 'confirmNext' };
    case 'Escape':
      return { type: 'back' };
    case ' ':
      return { type: 'playPause' };
    case 'Home':
      return { type: 'toBarOne' };
    case 'ArrowLeft':
      return { type: 'pan', bars: e.shiftKey ? -8 : -1 };
    case 'ArrowRight':
      return { type: 'pan', bars: e.shiftKey ? 8 : 1 };
    case 'ArrowUp':
    case 'ArrowDown':
      return e.shiftKey ? { type: 'volume', step: e.key === 'ArrowUp' ? 1 : -1 } : null;
    case '+':
    case '=':
      return { type: 'zoom', factor: 1.5 };
    case '-':
      return { type: 'zoom', factor: 1 / 1.5 };
  }
  // Option+letter types a symbol on a Mac (Option+D is `∂`): not a letter key here.
  switch (e.altKey ? e.key.toLowerCase() : letterOf(e)) {
    case 'n':
      return { type: 'nextReview' };
    case 'c':
      return { type: 'click' };
    case 'a':
      return { type: 'ghost' };
    case 'd':
      return { type: 'barOneHere', free: e.shiftKey };
    case 't':
      return { type: 'tap' };
    case 'i':
      return { type: 'details' };
    case 'm':
      return { type: 'meter' };
    case 'r':
      return { type: 'reset' };
    case 'b':
      return e.shiftKey ? { type: 'match' } : { type: 'version' };
    default:
      return null;
  }
}

/**
 * The letter a key stands for: the one typed, or, when it types a letter of another script
 * (Cyrillic, Greek), the one at its place (`KeyB`); else the key itself, lower-cased.
 */
function letterOf(e: KeyLike): string {
  if (/^[a-z]$/i.test(e.key)) return e.key.toLowerCase();
  const place = /^Key([A-Z])$/.exec(e.code)?.[1];
  // Only a letter of another script: Turkish ı or German ä are letters typed, not keys' places.
  const otherScript = /^\p{L}$/u.test(e.key) && !/\p{Script=Latin}/u.test(e.key);
  return place && otherScript ? place.toLowerCase() : e.key.toLowerCase();
}
