import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { Drawer } from './Drawer';

function Harness({ onWindowEscape }: { onWindowEscape: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div
      onKeyDown={(e) => {
        if (e.key === 'Escape') onWindowEscape();
      }}
    >
      <button type="button" onClick={() => setOpen(true)}>
        Settings
      </button>
      {open && (
        <Drawer label="Settings" onClose={() => setOpen(false)}>
          <button type="button">DJ</button>
          <input aria-label="Target" />
        </Drawer>
      )}
    </div>
  );
}

describe('the drawer', () => {
  it('takes focus, keeps Tab inside, closes on Esc alone and gives focus back', () => {
    const windowEscape = vi.fn();
    render(<Harness onWindowEscape={windowEscape} />);
    const trigger = screen.getByRole('button', { name: 'Settings' });
    trigger.focus();
    fireEvent.click(trigger);
    const dialog = screen.getByRole('dialog', { name: 'Settings' });
    const dj = screen.getByRole('button', { name: 'DJ' });
    const target = screen.getByLabelText('Target');
    expect(dj).toHaveFocus();
    fireEvent.keyDown(target, { key: 'Tab' });
    target.focus();
    fireEvent.keyDown(target, { key: 'Tab' });
    expect(dj).toHaveFocus();
    fireEvent.keyDown(dj, { key: 'Tab', shiftKey: true });
    expect(target).toHaveFocus();
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(windowEscape).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();
  });

  it('closes on a press outside the panel', () => {
    render(<Harness onWindowEscape={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const backdrop = screen.getByRole('dialog').previousElementSibling as Element;
    fireEvent.mouseDown(backdrop);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('from the panel itself, Tab and Shift+Tab go into its controls', () => {
    render(<Harness onWindowEscape={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const dialog = screen.getByRole('dialog');
    dialog.focus();
    fireEvent.keyDown(dialog, { key: 'Tab', shiftKey: true });
    expect(screen.getByLabelText('Target')).toHaveFocus();
    dialog.focus();
    fireEvent.keyDown(dialog, { key: 'Tab' });
    expect(screen.getByRole('button', { name: 'DJ' })).toHaveFocus();
  });

  it('gives focus to returnFocus() when the element that had it is gone', () => {
    function Gone() {
      const [open, setOpen] = useState(false);
      const [opener, setOpener] = useState(true);
      return (
        <>
          {opener && (
            <button type="button" onClick={() => setOpen(true)}>
              Opener
            </button>
          )}
          <button type="button" id="rail">
            Rail
          </button>
          {open && (
            <Drawer
              label="Settings"
              onClose={() => setOpen(false)}
              returnFocus={() => document.getElementById('rail')}
            >
              <button
                type="button"
                onClick={() => {
                  // The window widens: the opener goes, and the drawer with it.
                  setOpener(false);
                  setOpen(false);
                }}
              >
                Widen
              </button>
            </Drawer>
          )}
        </>
      );
    }
    render(<Gone />);
    const opener = screen.getByRole('button', { name: 'Opener' });
    opener.focus();
    fireEvent.click(opener);
    expect(screen.getByRole('button', { name: 'Widen' })).toHaveFocus();
    fireEvent.click(screen.getByRole('button', { name: 'Widen' }));
    expect(screen.queryByRole('button', { name: 'Opener' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Rail' })).toHaveFocus();
  });
});
