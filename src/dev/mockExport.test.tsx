import { act, fireEvent, render, screen, within } from '@testing-library/react';
import App from '@/App';
import { addPaths, pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { installMockBackend } from './mockBackend';

const row = (name: string) => {
  const found = screen.getAllByRole('row').find((r) => r.textContent?.includes(name));
  if (!found) throw new Error(`no row ${name}`);
  return found;
};

const advance = (ms: number) => act(async () => vi.advanceTimersByTimeAsync(ms));

describe('the browser stand-in', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    );
    useRecovery.setState({ running: false });
    useSettings.setState({ place: 'inPlace' });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('runs a whole export through the screen: confirm, row states, footer links, revert', async () => {
    installMockBackend();
    render(<App />);
    await advance(100);
    await act(async () => addPaths(['/Users/mock/Music/Crate']));
    await advance(10_000);
    expect(pipeline().getSnapshot().value).toBe('idle');
    // Its Serato track switched the batch to Library; the scenario wants the cuts.
    await act(async () => useSettings.getState().setBatchMode('prepare', ['/Users/mock/Music/Crate']));

    fireEvent.click(screen.getByRole('button', { name: /^Export \d+/ }));
    await advance(500);
    const dialog = screen.getByRole('dialog', { name: 'Confirm export' });
    expect(within(dialog).getByText(/backups in ~\/Music\/SoundCheck Backups/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Positions shift/)).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: /^Export \d+ files$/ }));
    await advance(0);
    expect(pipeline().getSnapshot().value).toBe('exporting');
    expect(screen.queryByRole('dialog')).toBeNull();

    // The first file walks its stages.
    await advance(100);
    expect(within(row('Night Drive')).getByText(/^Processing/)).toBeInTheDocument();
    await advance(350);
    expect(within(row('Night Drive')).getByText('Writing')).toBeInTheDocument();
    await advance(350);
    expect(within(row('Night Drive')).getByText('Checking')).toBeInTheDocument();
    expect(screen.getByText(/^Exporting 0 \/ /)).toBeInTheDocument();

    await advance(20_000);
    expect(pipeline().getSnapshot().value).toBe('idle');
    const night = within(row('Night Drive'));
    expect(night.getByText('Done, verified')).toBeInTheDocument();
    expect(night.getByText('Grid checks out')).toBeInTheDocument();
    expect(night.getByText('shift')).toBeInTheDocument();
    expect(within(row('Quiet Classic')).getByText(/^Off by/)).toBeInTheDocument();
    expect(within(row('Big Room')).getByText('Check bar 1')).toBeInTheDocument();
    expect(within(row('Already There')).getByText('Reads differently')).toBeInTheDocument();
    expect(within(row('Radio Edit')).getByText('Grid: XML only (MP3)')).toBeInTheDocument();
    expect(within(row('Lossless Import')).getByText(/analysed only/)).toBeInTheDocument();
    expect(within(row('Breakbeat Tool')).getByText('Failed')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show in Finder' })).toBeInTheDocument();
    expect(screen.getByText(/Last export: /)).toBeInTheDocument();

    // Revert: a spinner on that row, nothing else held up; then the original's row.
    const id = Object.values(useLibrary.getState().rows).find((r) => r.entry.info.title === 'Night Drive')?.entry.fileId;
    fireEvent.click(within(row('Night Drive')).getByRole('button', { name: 'Revert to the original' }));
    await advance(100);
    expect(within(row('Night Drive')).getByText('Reverting')).toBeInTheDocument();
    expect(useLibrary.getState().rows[id ?? 0]?.reverting).toBe(true);
    await advance(3000);
    expect(within(row('Night Drive')).queryByText('Done, verified')).toBeNull();
    expect(useLibrary.getState().rows[id ?? 0]?.export).toBeUndefined();
  });
});
