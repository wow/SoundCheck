import { analysed, entry, plan } from '@/test/fixtures';
import { useLibrary } from '@/state/library';
import { gridKey, type KeyLike } from './keys';
import { nextReview, queuePlace, reviewQueue } from './navigate';
import { tapBpm, tapped } from './tap';
import { snapped } from './geometry';

const key = (k: Partial<KeyLike>): KeyLike => ({
  key: '',
  code: '',
  shiftKey: false,
  altKey: false,
  metaKey: false,
  ctrlKey: false,
  ...k,
});

describe('grid view keys', () => {
  it('map to actions, nudges and beat keys by physical key', () => {
    expect(gridKey(key({ key: 'Enter' }))).toEqual({ type: 'confirmNext' });
    expect(gridKey(key({ key: 'N', shiftKey: true }))).toEqual({ type: 'nextReview' });
    expect(gridKey(key({ key: ';', code: 'Comma', shiftKey: true }))).toEqual({
      type: 'nudge',
      direction: -1,
      unit: '10ms',
    });
    expect(gridKey(key({ key: '…', code: 'Period', altKey: true }))).toEqual({
      type: 'nudge',
      direction: 1,
      unit: 'beat',
    });
    expect(gridKey(key({ key: '"', code: 'Digit2' }))).toEqual({ type: 'beatOne', beat: 2 });
    expect(gridKey(key({ key: 'z', code: 'KeyZ', metaKey: true, shiftKey: true }))).toEqual({
      type: 'redo',
    });
    expect(gridKey(key({ key: 'D', shiftKey: true }))).toEqual({ type: 'barOneHere', free: true });
    expect(gridKey(key({ key: 'ArrowRight', shiftKey: true }))).toEqual({ type: 'pan', bars: 8 });
    expect(gridKey(key({ key: 'Home' }))).toEqual({ type: 'toBarOne' });
    expect(gridKey(key({ key: 'I', shiftKey: true }))).toEqual({ type: 'details' });
    // Cmd+Z by the letter: German (KeyY), French (KeyW), and a Cyrillic layout by place.
    expect(gridKey(key({ key: 'z', code: 'KeyY', metaKey: true }))).toEqual({ type: 'undo' });
    expect(gridKey(key({ key: 'Z', code: 'KeyW', metaKey: true, shiftKey: true }))).toEqual({
      type: 'redo',
    });
    expect(gridKey(key({ key: 'я', code: 'KeyZ', metaKey: true }))).toEqual({ type: 'undo' });
    expect(gridKey(key({ key: 'y', code: 'KeyZ', metaKey: true }))).toBeNull();
    expect(gridKey(key({ key: 'x' }))).toBeNull();
    // Option+letter types a symbol on a Mac: no action.
    expect(gridKey(key({ key: '∂', code: 'KeyD', altKey: true }))).toBeNull();
    expect(gridKey(key({ key: 'c', metaKey: true }))).toBeNull();
  });
});

describe('listening keys', () => {
  it('switch the version with B and match the level with Shift+B, on any layout', () => {
    expect(gridKey(key({ key: 'b', code: 'KeyB' }))).toEqual({ type: 'version' });
    expect(gridKey(key({ key: 'B', code: 'KeyB', shiftKey: true }))).toEqual({ type: 'match' });
    // By the letter typed where it is Latin: Dvorak types b on the key at N's place.
    expect(gridKey(key({ key: 'b', code: 'KeyN' }))).toEqual({ type: 'version' });
    expect(gridKey(key({ key: 'n', code: 'KeyB' }))).toEqual({ type: 'nextReview' });
    // A Cyrillic layout types и on that key: by its place.
    expect(gridKey(key({ key: 'и', code: 'KeyB' }))).toEqual({ type: 'version' });
    expect(gridKey(key({ key: 'И', code: 'KeyB', shiftKey: true }))).toEqual({ type: 'match' });
    expect(gridKey(key({ key: '∫', code: 'KeyB', altKey: true }))).toBeNull();
    // Turkish Q types ı on the key at I's place: a Latin letter of its own, so not Details.
    expect(gridKey(key({ key: 'ı', code: 'KeyI' }))).toBeNull();
    expect(gridKey(key({ key: 'ä', code: 'Quote' }))).toBeNull();
    // Cyrillic ш on the same key goes by its place, and Greek ρ on R's.
    expect(gridKey(key({ key: 'ш', code: 'KeyI' }))).toEqual({ type: 'details' });
    expect(gridKey(key({ key: 'ρ', code: 'KeyR' }))).toEqual({ type: 'reset' });
    expect(gridKey(key({ key: 'b', code: 'KeyB', metaKey: true }))).toBeNull();
  });

  it('step the volume with Shift+Up and Shift+Down only', () => {
    expect(gridKey(key({ key: 'ArrowUp', shiftKey: true }))).toEqual({ type: 'volume', step: 1 });
    expect(gridKey(key({ key: 'ArrowDown', shiftKey: true }))).toEqual({
      type: 'volume',
      step: -1,
    });
    expect(gridKey(key({ key: 'ArrowUp' }))).toBeNull();
    expect(gridKey(key({ key: 'ArrowDown' }))).toBeNull();
    // The number keys stay beat 1: nothing mutes from the keyboard.
    expect(gridKey(key({ key: '0', code: 'Digit0' }))).toBeNull();
  });
});

describe('the review queue', () => {
  it('walks the rows needing review in table order, wrapping', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2), entry(3), entry(4)]);
    lib.applyEvent(analysed(1, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    lib.applyEvent(analysed(2));
    lib.applyEvent(analysed(4, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    const { order, rows } = useLibrary.getState();
    expect(reviewQueue(order, rows)).toEqual([1, 4]);
    expect(nextReview(order, rows, 1)).toBe(4);
    expect(nextReview(order, rows, 4)).toBe(1);
    expect(nextReview(order, rows, 2)).toBe(4);
    expect(queuePlace([1, 4], 4)).toEqual({ index: 2, total: 2 });
    expect(queuePlace([1, 4], 2)).toBeNull();
    expect(nextReview([1], { 1: rows[1]! }, 1)).toBeNull();
  });
});

describe('tap tempo', () => {
  it('takes the median of the last taps from the fourth on and restarts after a pause', () => {
    let taps: number[] = [];
    for (const t of [0, 500, 1000]) taps = tapped(taps, t);
    expect(tapBpm(taps)).toBeNull();
    taps = tapped(taps, 1480);
    expect(tapBpm(taps)).toBeCloseTo(120, 5);
    taps = tapped(taps, 1990);
    expect(tapBpm(taps)).toBeCloseTo(120, 5);
    expect(tapped(taps, 5000)).toEqual([5000]);
  });
});

describe('snapping bar 1', () => {
  it('reaches for the nearest attack within 40 ms', () => {
    const onsets = new Float64Array([0.5, 1.0, 1.5]);
    expect(snapped(onsets, 44_100 * 1.03, 44_100)).toBe(44_100);
    expect(snapped(onsets, 44_100 * 1.25, 44_100)).toBe(Math.round(44_100 * 1.25));
    expect(snapped(new Float64Array(0), 123, 44_100)).toBe(123);
  });
});
