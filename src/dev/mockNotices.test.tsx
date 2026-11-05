import { act, fireEvent, render, screen, within } from '@testing-library/react';
import App from '@/App';
import { addPaths, pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { resetRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { installMockBackend } from './mockBackend';

const advance = (ms: number) => act(async () => vi.advanceTimersByTimeAsync(ms));

function select(title: string) {
  const id = Object.values(useLibrary.getState().rows).find((r) => r.entry.info.title === title)
    ?.entry.fileId;
  act(() => useLibrary.getState().select(id ?? null));
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
  resetRecovery();
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// Whole flows on fake timers: quick in time, but each renders the app many times over.
describe('the browser stand-in, for the notices and the recovery banner', { timeout: 20_000 }, () => {
  it('exports rows with stale tags, Serato Autotags and a FLAC, each saying so', async () => {
    installMockBackend('none');
    render(<App />);
    await advance(100);
    await act(async () => addPaths(['/Users/mock/Music/Crate']));
    await advance(10_000);
    act(() => useSettings.getState().setOutDir('/Users/mock/Music/Exports'));
    fireEvent.click(screen.getByRole('button', { name: /^Export \d+/ }));
    await advance(500);
    const dialog = screen.getByRole('dialog', { name: 'Confirm export' });
    expect(
      within(dialog).getByText(/FLAC files in rekordbox once they are written/),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: /^Write \d+ files$/ }));
    await advance(30_000);
    expect(pipeline().getSnapshot().value).toBe('idle');

    select('Quiet Classic');
    const details = () => screen.getByRole('region', { name: 'Export details' });
    expect(
      within(details()).getByText(/^Album ReplayGain, RVA2 and iTunNORM were left as they are/),
    ).toBeInTheDocument();
    select('Serato Cued');
    expect(
      within(details()).getByText(
        /^Serato's Auto Gain for this file was measured at the old level/,
      ),
    ).toBeInTheDocument();
    select('Night Drive');
    expect(within(details()).getByText(/^Analyse this FLAC in rekordbox/)).toBeInTheDocument();
    expect(screen.getByText(/keep loudness tags at the old level/)).toBeInTheDocument();
  });

  it('?demo=recovery: the check runs, then two changes are left with Forget', async () => {
    installMockBackend('pending');
    render(<App />);
    await advance(800);
    expect(screen.getByText('Finishing changes from the last session…')).toBeInTheDocument();
    await advance(3000);
    const banner = screen.getByRole('region', { name: 'Changes from the last session' });
    expect(
      within(banner).getByText('2 changes from the last session could not be finished'),
    ).toBeInTheDocument();
    await act(async () => addPaths(['/Users/mock/Music/Crate']));
    await advance(10_000);
    fireEvent.click(
      within(banner).getAllByRole('button', { name: /^Forget the change to/ })[0] as HTMLElement,
    );
    // While the step asks, the table's keys wait: the arrows do not move the selection.
    const before = useLibrary.getState().selected;
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(useLibrary.getState().selected).toBe(before);
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Forget' }));
    await advance(100);
    expect(
      screen.getByText('1 change from the last session could not be finished'),
    ).toBeInTheDocument();
    expect(screen.getByText(/is forgotten\. Nothing was deleted\./)).toBeInTheDocument();
  });

  it('?demo=recovery-failed and ?demo=recovery-finished', async () => {
    installMockBackend('failed');
    const { unmount } = render(<App />);
    await advance(100);
    expect(screen.getByRole('alert')).toHaveTextContent(/could not be checked.*Permission denied/);
    unmount();
    resetRecovery();
    installMockBackend('finished');
    render(<App />);
    await advance(100);
    expect(
      screen.getByText(/^2 changes from the last session were settled: 1 finished, 1 undone/),
    ).toBeInTheDocument();
  });
});
