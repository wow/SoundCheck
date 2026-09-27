import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import { useTrack } from '@/features/grid/store';
import { useLibrary } from '@/state/library';
import { usePanels } from '@/state/panels';
import { useSettings } from '@/state/settings';
import App from './App';

describe('App', () => {
  it('shows the three promises and the version from the Rust side', async () => {
    mockIPC((cmd) => (cmd === 'app_version' ? '0.0.1' : undefined));
    render(<App />);
    expect(screen.getByRole('heading', { name: 'Same loudness' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Fits the grid' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Nothing lost' })).toBeInTheDocument();
    expect(await screen.findByText('v0.0.1')).toBeInTheDocument();
  });

  it('restores the list once, so leaving the grid view keeps the selection', async () => {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    );
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return undefined;
    });
    render(<App />);
    await waitFor(() => expect(calls).toContain('restore_session'));
    act(() => useLibrary.getState().select(7));
    act(() => useTrack.setState({ fileId: 7 }));
    act(() => useTrack.setState({ fileId: null }));
    await act(async () => {});
    expect(calls.filter((c) => c === 'restore_session')).toHaveLength(1);
    expect(useLibrary.getState().selected).toBe(7);
    vi.unstubAllGlobals();
  });
});

describe('settings in a narrow window', () => {
  let narrow = true;
  const changed = new Set<() => void>();
  const initialSettings = useSettings.getState();
  beforeEach(() => {
    narrow = true;
    vi.stubGlobal('matchMedia', (media: string) => ({
      get matches() {
        return narrow;
      },
      media,
      addEventListener: (_: string, f: () => void) => changed.add(f),
      removeEventListener: (_: string, f: () => void) => changed.delete(f),
    }));
    mockIPC(() => undefined);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    changed.clear();
    usePanels.setState({ settings: false });
    useSettings.setState(initialSettings, true);
    useLibrary.setState({ filter: 'all' });
  });
  const cmdComma = () => fireEvent.keyDown(window, { key: ',', metaKey: true });
  const widen = () =>
    act(() => {
      narrow = false;
      changed.forEach((f) => f());
    });

  it('opens by its button or Cmd+,, and gives way to the rail when the window widens', () => {
    render(<App />);
    // Narrow: no rail beside the table, a button naming the target instead.
    expect(screen.queryByRole('heading', { name: 'Loudness' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /^Settings:\s*DJ · −11\.0 LUFS$/ }));
    const drawer = screen.getByRole('dialog', { name: 'Settings' });
    expect(drawer).toContainElement(screen.getByRole('heading', { name: 'Loudness' }));
    cmdComma();
    expect(screen.queryByRole('dialog')).toBeNull();
    cmdComma();
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    // Widened: the rail is back, the drawer gone, and focus on the rail rather than nowhere.
    widen();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.queryByRole('button', { name: /LUFS$/ })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Loudness' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'DJ' })).toHaveFocus();
  });

  it('keeps the table keys while open: Esc closes it without clearing the filter', () => {
    render(<App />);
    act(() => useLibrary.getState().setFilter('review'));
    cmdComma();
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useLibrary.getState().filter).toBe('review');
  });

  it('saves a typed target when it closes, and Esc in the field drops the typing first', () => {
    render(<App />);
    cmdComma();
    const target = screen.getByLabelText(/Target/);
    target.focus();
    fireEvent.change(target, { target: { value: '-9' } });
    fireEvent.keyDown(target, { key: 'Escape' });
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(useSettings.getState().target).toBe(-11);
    expect(target).toHaveValue('-11.0');
    target.focus();
    fireEvent.change(target, { target: { value: '-9' } });
    cmdComma();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useSettings.getState().target).toBe(-9);
  });

  it('moves focus to the rail on Cmd+, in a wide window', () => {
    narrow = false;
    render(<App />);
    cmdComma();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: 'DJ' })).toHaveFocus();
  });
});
