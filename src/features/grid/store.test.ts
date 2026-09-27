import type * as Ipc from '@/lib/ipc';
import type { GridEdit, GridFit, RowUpdate, TrackEvent, TrackOpened } from '@/lib/ipc';
import { gridCommit, gridRefit, trackClose, trackOpen } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { analysis, entry, plan } from '@/test/fixtures';
import { tap } from './actions';
import { NO_EDIT, nudged, octaveStep, typedBpm } from './edit';
import { SAVE_DELAY_MS, useTrack } from './store';
import { testGrid } from './testGrid';

vi.mock('@/lib/ipc', async (original) => ({
  ...(await original<typeof Ipc>()),
  trackOpen: vi.fn(),
  trackClose: vi.fn(async () => {}),
  trackOnsets: vi.fn(async () => new Float64Array([0.5, 1.0])),
  gridRefit: vi.fn(),
  gridCommit: vi.fn(),
  playerSetClick: vi.fn(async () => {}),
  playerPlay: vi.fn(async () => {}),
  playerPause: vi.fn(async () => {}),
  playerSeek: vi.fn(async () => {}),
}));

const RATE = 48_000;
const analysed = testGrid(120, 24_000);

function opened(edit: GridEdit = NO_EDIT, fileId = 1): TrackOpened {
  return {
    entry: entry(fileId),
    sampleRate: RATE,
    frames: RATE * 200,
    gain: -3.2,
    bpmRange: [70, 180],
    analysed,
    grid: analysed,
    edit,
    confirmed: false,
    timeline: { hopMs: 100, shortTerm: [] },
    cover: null,
  };
}

function fit(bpm: number, anchor = 24_000): GridFit {
  return {
    header: {
      grid: testGrid(bpm, anchor),
      firstLine: 0,
      lines: 2,
      worstLine: null,
      matched: 2,
      attacks: 2,
    },
    residuals: new Float32Array([1, -2]),
  };
}

function update(confirmed: boolean): RowUpdate {
  return {
    fileId: 1,
    row: { ...analysis(), edited: true, confirmed },
    plan: plan({ status: 'analysed' }),
    revision: 0,
  };
}

const initialTrack = useTrack.getState();
const initialLibrary = useLibrary.getState();
beforeEach(() => {
  useTrack.setState(initialTrack, true);
  useLibrary.setState(initialLibrary, true);
  vi.mocked(trackOpen).mockReset().mockResolvedValue(opened());
  vi.mocked(gridRefit).mockReset().mockResolvedValue(fit(120));
  vi.mocked(gridCommit).mockReset().mockResolvedValue(update(false));
  vi.mocked(trackClose).mockReset().mockResolvedValue(undefined);
});

function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

const order = (fn: { mock: { invocationCallOrder: number[] } }) => fn.mock.invocationCallOrder;
afterEach(() => vi.useRealTimers());

describe('grid view store', () => {
  it('opens a track with its saved edit and refits it', async () => {
    const saved = octaveStep(NO_EDIT, analysed, 1);
    vi.mocked(trackOpen).mockResolvedValue(opened(saved));
    vi.mocked(gridRefit).mockResolvedValue(fit(240));
    await useTrack.getState().open(1);
    const s = useTrack.getState();
    expect(s.phase).toBe('open');
    expect(s.edits.present).toEqual(saved);
    expect(vi.mocked(gridRefit)).toHaveBeenCalledWith(1, saved);
    expect(s.grid?.bpm).toBe(240);
    expect(s.fit?.residuals.length).toBe(2);
  });

  it('shows a nudge at once and drops a refit answer that arrives after a newer edit', async () => {
    await useTrack.getState().open(1);
    let answerFirst: (f: GridFit) => void = () => {};
    vi.mocked(gridRefit)
      .mockImplementationOnce(() => new Promise((r) => (answerFirst = r)))
      .mockResolvedValueOnce(fit(120, 24_096));
    const first = nudged(NO_EDIT, analysed, 48);
    useTrack.getState().edit(first, { ...analysed, anchor: 24_048 });
    expect(useTrack.getState().grid?.anchor).toBe(24_048);
    useTrack
      .getState()
      .edit(nudged(first, testGrid(120, 24_048), 48), { ...analysed, anchor: 24_096 });
    await vi.waitFor(() => expect(useTrack.getState().grid?.anchor).toBe(24_096));
    answerFirst(fit(120, 24_048));
    await Promise.resolve();
    expect(useTrack.getState().grid?.anchor).toBe(24_096);
    useTrack.getState().undo();
    expect(useTrack.getState().edits.present).toEqual(first);
  });

  it('saves an edit unconfirmed after a pause, and confirmed on confirm', async () => {
    useLibrary.getState().add([entry(1)]);
    await useTrack.getState().open(1);
    vi.useFakeTimers();
    useTrack.getState().edit(octaveStep(NO_EDIT, analysed, 1));
    expect(useTrack.getState().saving).toBe(true);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS - 50);
    expect(vi.mocked(gridCommit)).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(100);
    expect(vi.mocked(gridCommit)).toHaveBeenCalledWith(1, octaveStep(NO_EDIT, analysed, 1), false);
    vi.useRealTimers();
    vi.mocked(gridCommit).mockResolvedValue(update(true));
    expect(await useTrack.getState().confirm()).toBe(true);
    expect(vi.mocked(gridCommit)).toHaveBeenLastCalledWith(
      1,
      octaveStep(NO_EDIT, analysed, 1),
      true,
    );
    expect(useTrack.getState().confirmed).toBe(true);
    expect(useLibrary.getState().rows[1]?.analysis?.confirmed).toBe(true);
    expect(useLibrary.getState().rows[1]?.state).toBe('analysed');
  });

  it('saves a waiting edit before closing, leaves at once and ignores edits after', async () => {
    await useTrack.getState().open(1);
    vi.useFakeTimers();
    const up = octaveStep(NO_EDIT, analysed, 1);
    useTrack.getState().edit(up);
    const closing = useTrack.getState().close();
    expect(useTrack.getState().phase).toBe('closed');
    useTrack.getState().edit(octaveStep(NO_EDIT, analysed, -1));
    await closing;
    expect(vi.mocked(gridCommit).mock.calls).toEqual([[1, up, false]]);
    expect(order(vi.mocked(gridCommit))[0]).toBeLessThan(order(vi.mocked(trackClose))[0] ?? 0);
  });

  it('opens one track after another: the last open wins, a stale answer is dropped', async () => {
    const first = deferred<TrackOpened>();
    vi.mocked(trackOpen)
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(opened(NO_EDIT, 2));
    const a = useTrack.getState().open(1);
    const b = useTrack.getState().open(2);
    await Promise.resolve();
    expect(vi.mocked(trackOpen)).toHaveBeenCalledTimes(1);
    first.resolve(opened());
    await Promise.all([a, b]);
    expect(vi.mocked(trackOpen).mock.calls.map((c) => c[0])).toEqual([1, 2]);
    expect(useTrack.getState()).toMatchObject({ fileId: 2, phase: 'open' });
    expect(vi.mocked(gridRefit).mock.calls.map((c) => c[0])).toEqual([2]);
  });

  it('closes a track that finishes opening after Esc', async () => {
    const first = deferred<TrackOpened>();
    vi.mocked(trackOpen).mockImplementationOnce(() => first.promise);
    const opening = useTrack.getState().open(1);
    const closing = useTrack.getState().close();
    expect(vi.mocked(trackClose)).not.toHaveBeenCalled();
    first.resolve(opened());
    await Promise.all([opening, closing]);
    expect(order(vi.mocked(trackClose))[0]).toBeGreaterThan(
      order(vi.mocked(trackOpen))[0] ?? Infinity,
    );
    expect(useTrack.getState().phase).toBe('closed');
    expect(vi.mocked(gridRefit)).not.toHaveBeenCalled();
  });

  it('neither confirms nor edits while the track opens', async () => {
    const first = deferred<TrackOpened>();
    vi.mocked(trackOpen).mockImplementationOnce(() => first.promise);
    const opening = useTrack.getState().open(1);
    expect(await useTrack.getState().confirm()).toBe(false);
    useTrack.getState().edit(octaveStep(NO_EDIT, analysed, 1));
    expect(useTrack.getState().edits.present).toEqual(NO_EDIT);
    first.resolve(opened());
    await opening;
    expect(vi.mocked(gridCommit)).not.toHaveBeenCalled();
  });

  it('shows a failed save, and saves again on retry', async () => {
    await useTrack.getState().open(1);
    vi.mocked(gridCommit).mockRejectedValueOnce({
      kind: 'io',
      message: 'no place to save grid edits',
    });
    vi.useFakeTimers();
    useTrack.getState().edit(octaveStep(NO_EDIT, analysed, 1));
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS + 10);
    expect(useTrack.getState()).toMatchObject({
      saving: false,
      saveError: 'no place to save grid edits',
    });
    vi.useRealTimers();
    useTrack.getState().retrySave();
    await vi.waitFor(() => expect(useTrack.getState().saveError).toBeNull());
    expect(useTrack.getState().saving).toBe(false);
  });

  it('saves in the order asked, so a confirmation is not overwritten by an earlier save', async () => {
    await useTrack.getState().open(1);
    const slow = deferred<RowUpdate>();
    vi.mocked(gridCommit)
      .mockImplementationOnce(() => slow.promise)
      .mockResolvedValueOnce(update(true));
    vi.useFakeTimers();
    useTrack.getState().edit(octaveStep(NO_EDIT, analysed, 1));
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS + 10);
    vi.useRealTimers();
    const confirming = useTrack.getState().confirm();
    slow.resolve(update(false));
    expect(await confirming).toBe(true);
    expect(vi.mocked(gridCommit).mock.calls.map((c) => c[2])).toEqual([false, true]);
    expect(useTrack.getState()).toMatchObject({ confirmed: true, saving: false });
  });

  it('drops the events of a closed track', async () => {
    let send: (e: TrackEvent) => void = () => {};
    vi.mocked(trackOpen).mockImplementationOnce(async (_fileId, onEvent) => {
      send = onEvent;
      return opened();
    });
    await useTrack.getState().open(1);
    send({ type: 'decoded', frames: 10 });
    expect(useTrack.getState().decoded).toBe(10);
    await useTrack.getState().close();
    send({ type: 'player', playing: true, position: 5, underruns: 0 });
    expect(useTrack.getState().player.playing).toBe(false);
  });

  it('keeps the grid shown, and says so, when an edit gives no grid', async () => {
    await useTrack.getState().open(1);
    vi.mocked(gridRefit).mockResolvedValueOnce({
      header: { ...fit(120).header, grid: null, lines: 0 },
      residuals: new Float32Array(0),
    });
    useTrack.getState().edit(typedBpm(NO_EDIT, analysed, 999));
    await vi.waitFor(() => expect(useTrack.getState().refitNote).toMatch(/no grid/));
    expect(useTrack.getState().grid?.bpm).toBe(120);
  });

  it('makes one undo step of a tap run', async () => {
    await useTrack.getState().open(1);
    // Slowing taps: every tap from the 4th gives a new tempo, all in one step.
    [0, 500, 1050, 1650, 2300, 3000].forEach((ms) => tap(10_000 + ms));
    const s = useTrack.getState();
    expect(s.edits.present.tempoHint).toBeCloseTo(100, 5);
    expect(s.edits.past).toEqual([NO_EDIT]);
  });

  it('follows decoding and the player from the event channel', async () => {
    await useTrack.getState().open(1);
    const s = useTrack.getState();
    s.handle({ type: 'decoded', frames: 65_536 });
    expect(useTrack.getState().decoded).toBe(65_536);
    s.handle({ type: 'ready', frames: RATE * 200 });
    expect(useTrack.getState().decodeDone).toBe(true);
    s.handle({ type: 'player', playing: true, position: 96_000, underruns: 0 });
    expect(useTrack.getState().player).toMatchObject({ playing: true, position: 96_000 });
    s.handle({ type: 'playerError', message: 'Output device changed. Press Space to resume.' });
    expect(useTrack.getState().player).toMatchObject({
      playing: false,
      error: expect.stringContaining('Space'),
    });
  });
});
