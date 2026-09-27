import { act, fireEvent, render, screen } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import App from '@/App';
import { useLibrary } from '@/state/library';
import { usePanels } from '@/state/panels';
import { entry } from '@/test/fixtures';

/** A window narrower than 1000 px. */
function narrowWindow() {
  vi.stubGlobal('matchMedia', (media: string) => ({
    matches: true,
    media,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

const initialLibrary = useLibrary.getState();
beforeEach(() => {
  narrowWindow();
  mockIPC(() => undefined);
});
afterEach(() => {
  vi.unstubAllGlobals();
  usePanels.setState({ settings: false, search: false });
  useLibrary.setState(initialLibrary, true);
});

describe('the toolbar in a narrow window', () => {
  it('folds the name filter into an icon that opens it in the chips’ place', () => {
    render(<App />);
    expect(screen.getByRole('button', { name: /^Review/ })).toHaveAttribute(
      'title',
      'Needs review',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Filter by name' }));
    const field = screen.getByRole('searchbox', { name: /^Filter by name/ });
    expect(field).toHaveFocus();
    expect(screen.queryByRole('navigation', { name: 'Filter tracks' })).toBeNull();
    // Left empty, it folds back and the chips return.
    fireEvent.blur(field);
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(screen.getByRole('navigation', { name: 'Filter tracks' })).toBeInTheDocument();
  });

  it('opens and focuses the name filter on Cmd+F, and keeps it open while it holds a name', () => {
    render(<App />);
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    const field = screen.getByRole('searchbox', { name: /^Filter by name/ });
    expect(field).toHaveFocus();
    fireEvent.change(field, { target: { value: 'drive' } });
    fireEvent.blur(field);
    expect(screen.getByRole('searchbox')).toHaveValue('drive');
    // Esc in the field clears it; then it folds.
    fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
    expect(useLibrary.getState().query).toBe('');
    expect(screen.queryByRole('searchbox')).toBeNull();
  });

  it('puts Add folder and Add tracks in one menu, and leaves the key hints out', () => {
    act(() => useLibrary.getState().add([entry(1)]));
    render(<App />);
    expect(screen.queryByText('next needs review')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Add folder' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    const menu = screen.getByRole('menu', { name: 'Add' });
    const items = screen.getAllByRole('menuitem');
    expect(items.map((i) => i.textContent)).toEqual(['Add folder…', 'Add tracks…']);
    expect(items[0]).toHaveFocus();
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(items[1]).toHaveFocus();
    fireEvent.keyDown(menu, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
  });
});
