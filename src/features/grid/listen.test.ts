import type * as Ipc from '@/lib/ipc';
import type { Listen, MeterFrame, TrackEvent, TrackOpened } from '@/lib/ipc';
import { gridPlayerListen, trackOpen } from '@/lib/ipc';
import { useSettings } from '@/state/settings';
import { entry } from '@/test/fixtures';
import { switchVersion, toggleMatch } from './actions';
import { NO_EDIT } from './edit';
import { LISTEN_GRACE_MS, OPEN_LISTEN, useTrack } from './store';
import { testGrid } from './testGrid';

vi.mock('@/lib/ipc', async (original) => ({
  ...(await original<typeof Ipc>()),
  trackOpen: vi.fn(),
  trackClose: vi.fn(async () => {}),
  trackOnsets: vi.fn(async () => new Float64Array(0)),
  gridRefit: vi.fn(async () => {
    throw new Error('not needed');
  }),
  gridPlayerListen: vi.fn(async () => {}),
}));

const grid = testGrid(120, 24_000);
const opened: TrackOpened = {
  entry: entry(1),
  sampleRate: 48_000,
  frames: 48_000 * 200,
  gain: -2.3,
  bpmRange: [70, 180],
  analysed: grid,
  grid,
  edit: NO_EDIT,
  confirmed: false,
  timeline: { hopMs: 100, shortTerm: [] },
  cover: null,
};

function frame(inPeak: number, gain = -2.3): MeterFrame {
  return {
    position: 96_000,
    inPeak,
    inMomentary: -9,
    outPeak: inPeak + gain,
    outMomentary: -9 + gain,
    folded: false,
  };
}

function player(listen: Listen, meter: MeterFrame | null = null, playing = true): TrackEvent {
  return { type: 'player', playing, position: 96_000, underruns: 0, meter, listen };
}

function deferred() {
  let resolve!: () => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const initialTrack = useTrack.getState();
const initialSettings = useSettings.getState();
afterEach(() => vi.restoreAllMocks());
beforeEach(async () => {
  useTrack.setState(initialTrack, true);
  useSettings.setState(initialSettings, true);
  vi.mocked(trackOpen).mockReset().mockResolvedValue(opened);
  vi.mocked(gridPlayerListen).mockReset().mockResolvedValue(undefined);
  await useTrack.getState().open(1);
});

describe('the version heard', () => {
  it('opens on the processed version and switches with B and Shift+B at once', async () => {
    expect(useTrack.getState().listen).toEqual(OPEN_LISTEN);
    switchVersion();
    expect(useTrack.getState().listen).toEqual({ version: 'original', matched: false });
    toggleMatch();
    expect(useTrack.getState().listen).toEqual({ version: 'original', matched: true });
    await vi.waitFor(() => expect(vi.mocked(gridPlayerListen)).toHaveBeenCalledTimes(2));
    expect(vi.mocked(gridPlayerListen).mock.calls).toEqual([
      [1, { version: 'original', matched: false }],
      [1, { version: 'original', matched: true }],
    ]);
  });

  it('keeps a switch on screen until a report shows it, or its grace runs out', async () => {
    let now = 1000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
    const slow = deferred();
    vi.mocked(gridPlayerListen).mockImplementationOnce(() => slow.promise);
    const t = () => useTrack.getState();
    switchVersion();
    // Reports sent before the switch reached the player.
    t().handle(player(OPEN_LISTEN));
    expect(t().listen.version).toBe('original');
    slow.resolve();
    await vi.waitFor(() => expect(vi.mocked(gridPlayerListen)).toHaveBeenCalled());
    await Promise.resolve();
    await Promise.resolve();
    // Landed, but the player's next report can still predate it: no flash back.
    now += LISTEN_GRACE_MS - 10;
    t().handle(player(OPEN_LISTEN));
    expect(t().listen.version).toBe('original');
    // A report that shows it settles it; from then on the player is the authority.
    t().handle(player({ version: 'original', matched: false }));
    t().handle(player(OPEN_LISTEN));
    expect(t().listen).toEqual(OPEN_LISTEN);
  });

  it('never flashes back on a quick B, B whose reports arrive out of step', async () => {
    const first = deferred();
    const second = deferred();
    vi.mocked(gridPlayerListen)
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const t = () => useTrack.getState();
    switchVersion();
    switchVersion();
    expect(t().listen).toEqual(OPEN_LISTEN);
    // A report from before either switch matches the second; then the first lands and reports.
    t().handle(player(OPEN_LISTEN));
    t().handle(player({ version: 'original', matched: false }));
    expect(t().listen).toEqual(OPEN_LISTEN);
    first.resolve();
    second.resolve();
  });

  it('takes the player’s word once the grace after a landed switch runs out', async () => {
    let now = 1000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
    switchVersion();
    await vi.waitFor(() => expect(vi.mocked(gridPlayerListen)).toHaveBeenCalled());
    await Promise.resolve();
    await Promise.resolve();
    now += LISTEN_GRACE_MS + 1;
    useTrack.getState().handle(player(OPEN_LISTEN));
    expect(useTrack.getState().listen).toEqual(OPEN_LISTEN);
  });

  it('follows a switch reported while paused, made elsewhere', () => {
    const elsewhere = { version: 'original', matched: true } as const;
    useTrack.getState().handle(player(elsewhere, null, false));
    expect(useTrack.getState().listen).toEqual(elsewhere);
  });

  it('goes back to the version before when the player refuses the switch', async () => {
    vi.mocked(gridPlayerListen).mockRejectedValueOnce({ message: 'no track is open' });
    switchVersion();
    await vi.waitFor(() => expect(useTrack.getState().player.error).toBe('no track is open'));
    expect(useTrack.getState().listen).toEqual(OPEN_LISTEN);
  });

  it('starts every newly opened track on the processed version', async () => {
    switchVersion();
    await useTrack.getState().open(1);
    expect(useTrack.getState().listen).toEqual(OPEN_LISTEN);
  });
});

describe('meter readings', () => {
  it('keeps the reading for the audio heard, and none once stopped', () => {
    const f = frame(-3);
    useTrack.getState().handle(player(OPEN_LISTEN, f));
    expect(useTrack.getState().meter).toBe(f);
    useTrack.getState().handle(player(OPEN_LISTEN, null, false));
    expect(useTrack.getState().meter).toBeNull();
  });

  it('latches an over until it is cleared or another track opens', async () => {
    useSettings.setState({ ceiling: -1 });
    const t = () => useTrack.getState();
    t().handle(player(OPEN_LISTEN, frame(-2)));
    expect(t().overs).toEqual({ in: false, out: false });
    // -0.2 dBTP in, -2.5 out: neither over.
    t().handle(player(OPEN_LISTEN, frame(-0.2)));
    expect(t().overs).toEqual({ in: false, out: false });
    // +0.4 in (over 0 dBTP); out at a +1 dB gain is +1.4 (over the -1 ceiling).
    t().handle(player(OPEN_LISTEN, frame(0.4, 1)));
    expect(t().overs).toEqual({ in: true, out: true });
    t().handle(player(OPEN_LISTEN, frame(-20)));
    expect(t().overs).toEqual({ in: true, out: true });
    t().clearOver('out');
    expect(t().overs).toEqual({ in: true, out: false });
    await t().open(1);
    expect(t().overs).toEqual({ in: false, out: false });
  });
});
