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
});
