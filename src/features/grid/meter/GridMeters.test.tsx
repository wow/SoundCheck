import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { MeterFrame, TrackOpened } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { useMonitor } from '@/state/monitor';
import { usePanels } from '@/state/panels';
import { useSettings } from '@/state/settings';
import { analysed, analysis, entry, plan } from '@/test/fixtures';
import { GridView } from '../GridView';
import { NO_EDIT } from '../edit';
import { OPEN_LISTEN, useTrack } from '../store';
import { testGrid } from '../testGrid';
import { MATCH_TIP } from './AbControl';

const grid = testGrid(124, 24_000);

function openTrack() {
  const opened: TrackOpened = {
    entry: entry(1, 'Night Drive'),
    sampleRate: 44_100,
    frames: 44_100 * 200,
    gain: -1.8,
    bpmRange: [70, 180],
    analysed: grid,
    grid,
    edit: NO_EDIT,
    confirmed: false,
    timeline: { hopMs: 100, shortTerm: [] },
    cover: null,
  };
  const lib = useLibrary.getState();
  lib.add([entry(1, 'Night Drive')]);
  lib.applyEvent(
    analysed(1, plan(), analysis({ shortTermP95: -9.2, integrated: -10.6, truePeak: -0.3 })),
  );
  useTrack.setState({ fileId: 1, opened, phase: 'open', grid });
}

let narrow = false;
let calls: { cmd: string; args: unknown }[] = [];
const initialTrack = useTrack.getState();
const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();
const initialMonitor = useMonitor.getState();
beforeEach(() => {
  narrow = false;
  calls = [];
  vi.stubGlobal('matchMedia', (media: string) => ({
    get matches() {
      return narrow && media.includes('max-width');
    },
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
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return undefined;
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  useTrack.setState(initialTrack, true);
  useLibrary.setState(initialLibrary, true);
  useSettings.setState(initialSettings, true);
  useMonitor.setState(initialMonitor, true);
  usePanels.setState({ details: false });
});

const meterGroup = (name: RegExp) => screen.getByRole('group', { name });
const frame: MeterFrame = {
  position: 44_100,
  inPeak: 0.4,
  inMomentary: -8.9,
  outPeak: -1.4,
  outMomentary: -10.7,
  folded: false,
};

describe('the meter strips', () => {
  it('stand either side of the waveform, 28 px wide, with the track’s numbers while stopped', () => {
    openTrack();
    render(<GridView />);
    const inStrip = meterGroup(/^IN meter/);
    const outStrip = meterGroup(/^OUT meter/);
    expect(inStrip.style.width).toBe('28px');
    expect(outStrip.style.width).toBe('28px');
    // IN, the waveform, OUT: in that order.
    const waveform = screen.getByRole('img', { name: /Waveform/ });
    expect(inStrip.compareDocumentPosition(waveform)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(outStrip.compareDocumentPosition(waveform)).toBe(Node.DOCUMENT_POSITION_PRECEDING);
    expect(inStrip).toHaveTextContent('S-P95 -9.2 · TP -0.3');
    expect(outStrip).toHaveTextContent('S-P95 -11.0 · TP -2.1');
    const tp = within(outStrip).getByRole('meter', { name: 'OUT true peak of the track' });
    expect(tp).toHaveAttribute('aria-valuetext', '-2.1 dBTP');
    expect(within(inStrip).getByRole('meter', { name: 'IN S-P95' })).toHaveAttribute(
      'aria-valuenow',
      '-9.2',
    );
  });

  it('caption the integrated loudness in Streaming mode', () => {
    useSettings.setState({ mode: 'streaming' });
    openTrack();
    render(<GridView />);
    expect(meterGroup(/^IN meter/)).toHaveTextContent('Integrated -10.6 · TP -0.3');
    expect(meterGroup(/^OUT meter/)).toHaveTextContent('Integrated -12.4 · TP -2.1');
  });

  it('fold to 12 px in a narrow window, the numbers in a tooltip', () => {
    narrow = true;
    openTrack();
    render(<GridView />);
    const outStrip = meterGroup(/^OUT meter/);
    expect(outStrip.style.width).toBe('12px');
    expect(outStrip).toHaveAttribute('title', 'OUT · S-P95 -11.0 · TP -2.1');
    // The readings stay for screen readers.
    expect(within(outStrip).getByRole('meter', { name: 'OUT S-P95' })).toBeInTheDocument();
  });

  it('dim the version not heard', () => {
    openTrack();
    render(<GridView />);
    expect(meterGroup(/^OUT meter/)).not.toHaveClass('opacity-[.45]');
    expect(meterGroup(/^IN meter/)).toHaveClass('opacity-[.45]');
    act(() => useTrack.setState({ listen: { version: 'original', matched: false } }));
    expect(meterGroup(/^IN meter/)).not.toHaveClass('opacity-[.45]');
    expect(meterGroup(/^OUT meter/)).toHaveClass('opacity-[.45]');
  });

  it('show an over until it is clicked', () => {
    openTrack();
    render(<GridView />);
    act(() =>
      useTrack.getState().handle({
        type: 'player',
        playing: true,
        position: 44_100,
        underruns: 0,
        meter: frame,
        listen: OPEN_LISTEN,
      }),
    );
    expect(useTrack.getState().overs).toEqual({ in: true, out: false });
    const clear = screen.getByRole('button', { name: 'IN peaked over 0 dBTP; clear' });
    fireEvent.click(clear);
    expect(useTrack.getState().overs.in).toBe(false);
    expect(screen.queryByRole('button', { name: /peaked over/ })).toBeNull();
  });
});

describe('a long track held as mono', () => {
  it('says so quietly on the readings while playing', async () => {
    openTrack();
    render(<GridView />);
    act(() =>
      useTrack.getState().handle({
        type: 'player',
        playing: true,
        position: 44_100,
        underruns: 0,
        meter: { ...frame, inPeak: -3, outPeak: -4.8, folded: true },
        listen: OPEN_LISTEN,
      }),
    );
    const outStrip = meterGroup(/^OUT meter/);
    await vi.waitFor(() =>
      expect(
        within(outStrip).getByRole('meter', { name: 'OUT true peak, mono sum' }),
      ).toHaveAttribute('aria-valuetext', '-4.8 dBTP'),
    );
    expect(outStrip.getAttribute('title')).toMatch(/^OUT · mono sum/);
  });
});

describe('the A/B control', () => {
  it('switches the version and the level match, by click and by key', () => {
    openTrack();
    render(<GridView />);
    const group = screen.getByRole('group', { name: 'Version heard' });
    const processed = within(group).getByRole('button', { name: 'Processed −1.8 dB' });
    const original = within(group).getByRole('button', { name: 'Original' });
    expect(processed).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(original);
    expect(original).toHaveAttribute('aria-pressed', 'true');
    fireEvent.keyDown(window, { key: 'b', code: 'KeyB' });
    expect(processed).toHaveAttribute('aria-pressed', 'true');
    const match = screen.getByRole('button', { name: 'Match level' });
    expect(match).toHaveAttribute('title', MATCH_TIP);
    expect(match).toHaveAttribute('aria-pressed', 'false');
    fireEvent.keyDown(window, { key: 'B', code: 'KeyB', shiftKey: true });
    expect(match).toHaveAttribute('aria-pressed', 'true');
    expect(useTrack.getState().listen).toEqual({ version: 'processed', matched: true });
  });
});

describe('the volume control', () => {
  it('sits right of play/pause, mutes from its icon and steps with Shift+Up/Down', () => {
    openTrack();
    render(<GridView />);
    const transport = screen.getByRole('group', { name: 'Transport' });
    const play = within(transport).getByRole('button', { name: 'Play' });
    const slider = within(transport).getByRole('slider', { name: 'Volume' });
    expect(play.compareDocumentPosition(slider)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(slider).toHaveAttribute('aria-valuetext', '0.0 dB');
    fireEvent.keyDown(window, { key: 'ArrowDown', shiftKey: true });
    fireEvent.keyDown(window, { key: 'ArrowDown', shiftKey: true, repeat: true });
    expect(slider).toHaveAttribute('aria-valuetext', '−2.0 dB');
    fireEvent.keyDown(window, { key: 'ArrowUp', shiftKey: true });
    expect(useMonitor.getState().db).toBe(-1);
    fireEvent.click(within(transport).getByRole('button', { name: 'Mute' }));
    expect(slider).toHaveAttribute('aria-valuetext', 'Muted · −1.0 dB');
    // The slider's own keys stay with it: Left steps the volume instead of panning.
    fireEvent.keyDown(slider, { key: 'ArrowLeft' });
    expect(useMonitor.getState()).toMatchObject({ db: -2, muted: false });
    fireEvent.keyDown(slider, { key: 'Home' });
    expect(slider).toHaveAttribute('aria-valuetext', 'Off');
  });

  it('is an icon with a popover in a narrow window', () => {
    narrow = true;
    openTrack();
    render(<GridView />);
    expect(screen.queryByRole('slider')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Volume' }));
    const pop = screen.getByRole('dialog', { name: 'Monitor volume' });
    const slider = within(pop).getByRole('slider', { name: 'Volume' });
    expect(slider).toHaveFocus();
    expect(pop).toHaveTextContent('0.0 dB');
    fireEvent.keyDown(slider, { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Monitor volume' })).toBeNull();
    // Esc closed the popover, not the grid view.
    expect(useTrack.getState().fileId).toBe(1);
  });
});
