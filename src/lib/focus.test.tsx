import { fireEvent, render, screen } from '@testing-library/react';
import { keepFocusOnClick, ownsKey } from './focus';

describe('focus and keys', () => {
  it('gives text fields every key and controls only Enter and Space', () => {
    render(
      <div>
        <input aria-label="bpm" />
        <button type="button">
          <span>Reset</span>
        </button>
        <div role="menuitem" tabIndex={-1}>
          4/4
        </div>
      </div>,
    );
    const input = screen.getByLabelText('bpm');
    expect(ownsKey(input, 'z')).toBe(true);
    expect(ownsKey(input, 'Enter')).toBe(true);
    const inner = screen.getByText('Reset');
    expect(ownsKey(inner, 'Enter')).toBe(true);
    expect(ownsKey(inner, ' ')).toBe(true);
    expect(ownsKey(inner, 'd')).toBe(false);
    expect(ownsKey(screen.getByRole('menuitem'), 'Enter')).toBe(true);
    expect(ownsKey(document.body, 'Enter')).toBe(false);
    expect(ownsKey(null, 'Enter')).toBe(false);
  });

  it('keeps focus where it was when a button is clicked', () => {
    let pressed = 0;
    render(
      <div onMouseDownCapture={keepFocusOnClick}>
        <button type="button" onClick={() => pressed++}>
          Play
        </button>
        <input aria-label="bpm" />
      </div>,
    );
    const button = screen.getByRole('button');
    expect(fireEvent.mouseDown(button)).toBe(false);
    fireEvent.click(button);
    expect(pressed).toBe(1);
    expect(fireEvent.mouseDown(screen.getByLabelText('bpm'))).toBe(true);
  });
});
