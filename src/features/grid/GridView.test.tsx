import { act, fireEvent, render, screen } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { TrackOpened } from '@/lib/ipc';
import { usePanels } from '@/state/panels';
import { entry } from '@/test/fixtures';
import { GridView } from './GridView';
import { NO_EDIT, octaveStep } from './edit';
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
  const edited = octaveStep(NO_EDIT, grid, 1);
  useTrack.setState({
    fileId: 1,
    opened,
    phase: 'open',
    grid,
    edits: { past: [NO_EDIT], present: edited, future: [] },
  });
}

/** A window whose width can change during a test. */
let narrow = true;
const changed = new Set<() => void>();
function resizeTo(isNarrow: boolean) {
  act(() => {
    narrow = isNarrow;
    changed.forEach((f) => f());
  });
}

const initialTrack = useTrack.getState();
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
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
  mockIPC(() => undefined);
});
afterEach(() => {
  vi.unstubAllGlobals();
  changed.clear();
  useTrack.setState(initialTrack, true);
  usePanels.setState({ details: false });
});

describe('the grid view in a narrow window', () => {
  it('shows the fit in one line, with the rest behind Details (I)', () => {
    openTrack();
    render(<GridView />);
    // No rail: the fit strip, its numbers coloured and worded against the verdict limits.
    const strip = screen.getByRole('group', { name: 'Grid fit' });
    expect(screen.queryByRole('region', { name: 'Grid fit' })).toBeNull();
    expect(strip).toHaveTextContent('250.00');
    expect(screen.getByText('14 ms')).toHaveClass('text-warn');
    expect(screen.getByText('52 ms')).toHaveClass('text-err');
    expect(strip).toHaveTextContent('Residual max 52 ms, over the limit');
    fireEvent.click(screen.getByRole('button', { name: 'Details' }));
    const drawer = screen.getByRole('dialog', { name: 'Grid details' });
    expect(drawer).toContainElement(screen.getByRole('region', { name: 'Grid fit' }));
  });

  it('keeps the drawer’s keys safe: focus on the panel, undo and play pass, fixes wait, Esc only closes', () => {
    openTrack();
    render(<GridView />);
    fireEvent.keyDown(window, { key: 'i' });
    const drawer = screen.getByRole('dialog', { name: 'Grid details' });
    // Not on the ½ button, which Space would press out of habit.
    expect(drawer).toHaveFocus();
    // A fix waits while it is open; undo goes through.
    fireEvent.keyDown(window, { key: 'r' });
    expect(useTrack.getState().edits.present.octave).toBe(1);
    fireEvent.keyDown(window, { key: 'z', code: 'KeyZ', metaKey: true });
    expect(useTrack.getState().edits.present.octave).toBe(0);
    // Esc closes the drawer and stays in the view.
    fireEvent.keyDown(drawer, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useTrack.getState().fileId).toBe(1);
    // Esc from outside the panel (focus lost to the page) does the same.
    fireEvent.keyDown(window, { key: 'i' });
    act(() => (document.activeElement as HTMLElement | null)?.blur());
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useTrack.getState().fileId).toBe(1);
  });

  it('closes the drawer when the window widens or the view is left', () => {
    openTrack();
    const { unmount } = render(<GridView />);
    fireEvent.keyDown(window, { key: 'i' });
    resizeTo(false);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(usePanels.getState().details).toBe(false);
    expect(screen.getByRole('region', { name: 'Grid fit' })).toBeInTheDocument();
    resizeTo(true);
    fireEvent.keyDown(window, { key: 'i' });
    expect(usePanels.getState().details).toBe(true);
    unmount();
    expect(usePanels.getState().details).toBe(false);
  });

  it('shows a failed save above the waveform, with Save again, where the rail is folded', () => {
    openTrack();
    act(() => useTrack.setState({ saveError: 'no place to save grid edits', saving: false }));
    render(<GridView />);
    expect(screen.getByRole('alert')).toHaveTextContent('Not saved: no place to save grid edits');
    expect(screen.getByRole('button', { name: 'Save again' })).toBeInTheDocument();
  });

  it('keeps the rail beside the waveform in a wide window, where I does nothing', () => {
    narrow = false;
    openTrack();
    render(<GridView />);
    expect(screen.getByRole('region', { name: 'Grid fit' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Details' })).toBeNull();
    fireEvent.keyDown(window, { key: 'i' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});
