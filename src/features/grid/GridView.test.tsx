import { fireEvent, render, screen } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { TrackOpened } from '@/lib/ipc';
import { usePanels } from '@/state/panels';
import { entry } from '@/test/fixtures';
import { GridView } from './GridView';
import { NO_EDIT, history } from './edit';
import { useTrack } from './store';
import { AKSAK, testGrid } from './testGrid';

const RATE = 48_000;
const grid = {
  ...testGrid(250, 24_000, AKSAK),
  residualP95Ms: 14,
  residualMaxMs: 52,
  verdict: 'drifts' as const,
};

function openTrack() {
  const opened: TrackOpened = {
    entry: entry(1, 'Skalonga'),
    sampleRate: RATE,
    frames: RATE * 200,
    gain: -3,
    bpmRange: [70, 180],
    analysed: grid,
    grid,
    edit: NO_EDIT,
    confirmed: false,
    timeline: { hopMs: 100, shortTerm: [] },
    cover: null,
  };
  useTrack.setState({ fileId: 1, opened, phase: 'open', grid, edits: history(NO_EDIT) });
}

function windowWidth(narrow: boolean) {
  vi.stubGlobal('matchMedia', (media: string) => ({
    matches: narrow,
    media,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
}

const initialTrack = useTrack.getState();
beforeEach(() => mockIPC(() => undefined));
afterEach(() => {
  vi.unstubAllGlobals();
  useTrack.setState(initialTrack, true);
  usePanels.setState({ details: false });
});

describe('the grid view in a narrow window', () => {
  it('shows the fit in one line, with the rest behind Details (I)', () => {
    windowWidth(true);
    openTrack();
    render(<GridView />);
    // No rail: the fit strip, its numbers coloured against the verdict limits.
    expect(screen.queryByRole('region', { name: 'Grid fit' })).toBeNull();
    expect(screen.getByText('250.00')).toBeInTheDocument();
    expect(screen.getByText('14 ms')).toHaveClass('text-warn');
    expect(screen.getByText('52 ms')).toHaveClass('text-err');
    expect(screen.getByText('Drifts', { selector: 'span' })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'i' });
    const drawer = screen.getByRole('dialog', { name: 'Grid details' });
    expect(drawer).toContainElement(screen.getByRole('region', { name: 'Grid fit' }));
    // While it is open the grid keys wait: C does not turn the click off.
    fireEvent.keyDown(window, { key: 'c' });
    expect(useTrack.getState().click).toBe(true);
    fireEvent.keyDown(window, { key: 'i' });
    expect(screen.queryByRole('dialog')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Details/ }));
    expect(screen.getByRole('dialog', { name: 'Grid details' })).toBeInTheDocument();
  });

  it('keeps the rail beside the waveform in a wide window, where I does nothing', () => {
    windowWidth(false);
    openTrack();
    render(<GridView />);
    expect(screen.getByRole('region', { name: 'Grid fit' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Details/ })).toBeNull();
    fireEvent.keyDown(window, { key: 'i' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
