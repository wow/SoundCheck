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

/** Focus moves away from the name filter, as a click or Tab elsewhere would. */
const leaveField = () => act(() => screen.getByRole('button', { name: 'Add' }).focus());

describe('the toolbar in a narrow window', () => {
  it('folds the name filter into an icon that opens it in the chips’ place', () => {
    render(<App />);
    // The short label on screen, the full one for screen readers and on hover.
    const review = screen.getByRole('button', { name: 'Needs review 0' });
    expect(review).toHaveTextContent('Review0');
    expect(review).toHaveAttribute('title', 'Needs review');
    fireEvent.click(screen.getByRole('button', { name: 'Filter by name' }));
    const field = screen.getByRole('searchbox', { name: /^Filter by name/ });
    expect(field).toHaveFocus();
    expect(screen.queryByRole('navigation', { name: 'Filter tracks' })).toBeNull();
    // Left empty, it folds back and the chips return.
    leaveField();
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(screen.getByRole('navigation', { name: 'Filter tracks' })).toBeInTheDocument();
  });

  it('opens and focuses the name filter on Cmd+F, and keeps it open while it holds a name', () => {
    render(<App />);
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    const field = screen.getByRole('searchbox', { name: /^Filter by name/ });
    expect(field).toHaveFocus();
    fireEvent.change(field, { target: { value: 'drive' } });
    leaveField();
    expect(screen.getByRole('searchbox')).toHaveValue('drive');
    // Esc in the field clears it; then it folds.
    act(() => screen.getByRole('searchbox').focus());
    fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
    expect(useLibrary.getState().query).toBe('');
    expect(screen.queryByRole('searchbox')).toBeNull();
  });

  it('folds the field when its name is cleared elsewhere, such as by Esc on the table', () => {
    render(<App />);
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'drive' } });
    leaveField();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(useLibrary.getState().query).toBe('');
    expect(screen.queryByRole('searchbox')).toBeNull();
    expect(screen.getByRole('navigation', { name: 'Filter tracks' })).toBeInTheDocument();
  });

  it('shows the chip still filtering while the field has the chips’ place', () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: /^Done/ }));
    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    const token = screen.getByRole('button', { name: 'Show all tracks, not only Done' });
    expect(token).toHaveTextContent('in Done');
    fireEvent.click(token);
    expect(useLibrary.getState().filter).toBe('all');
    expect(screen.queryByRole('button', { name: /Show all tracks/ })).toBeNull();
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
    expect(items.every((i) => i.tabIndex === -1)).toBe(true);
    expect(items[0]).toHaveFocus();
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(items[1]).toHaveFocus();
    // Esc closes it and gives focus back to its button.
    fireEvent.keyDown(menu, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(screen.getByRole('button', { name: 'Add' })).toHaveFocus();
  });
});
