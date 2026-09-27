import { act, render, screen, waitFor } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import { useTrack } from '@/features/grid/store';
import { useLibrary } from '@/state/library';
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
