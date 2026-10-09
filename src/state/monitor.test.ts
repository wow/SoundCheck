import type * as Ipc from '@/lib/ipc';
import type * as Persist from '@/lib/persist';
import { playerVolume } from '@/lib/ipc';
import { loadMonitor, saveMonitor } from '@/lib/persist';
import { MONITOR_SAVE_DELAY_MS, startMonitorSync } from '@/features/settings/monitorSync';
import { DEFAULT_MONITOR, heardDb, sanitizeMonitor, useMonitor } from './monitor';

vi.mock('@/lib/ipc', async (original) => ({
  ...(await original<typeof Ipc>()),
  playerVolume: vi.fn(async () => {}),
}));
vi.mock('@/lib/persist', async (original) => ({
  ...(await original<typeof Persist>()),
  loadMonitor: vi.fn(async () => ({})),
  saveMonitor: vi.fn(async () => {}),
}));

const initial = useMonitor.getState();
beforeEach(() => {
  useMonitor.setState(initial, true);
  vi.mocked(playerVolume).mockReset().mockResolvedValue(undefined);
  vi.mocked(saveMonitor).mockReset().mockResolvedValue(undefined);
  vi.mocked(loadMonitor).mockReset().mockResolvedValue({});
});
afterEach(() => vi.useRealTimers());

describe('the monitor volume', () => {
  it('starts at 0 dB, steps, mutes without moving the slider', () => {
    const m = () => useMonitor.getState();
    expect(heardDb(m())).toBe(0);
    m().step(-1);
    expect(m().db).toBe(-1);
    m().toggleMute();
    expect(m()).toMatchObject({ db: -1, muted: true });
    expect(heardDb(m())).toBeNull();
    // A step unmutes.
    m().step(-1);
    expect(m()).toMatchObject({ db: -2, muted: false });
    m().setDb(null);
    expect(heardDb(m())).toBeNull();
    // The speaker icon on a slider left at off brings it back up.
    m().toggleMute();
    expect(m()).toMatchObject({ db: -20, muted: false });
    m().setDb(6);
    expect(m().db).toBe(0);
  });

  it('keeps only valid saved values', () => {
    expect(sanitizeMonitor({ db: -12.5, muted: true })).toEqual({ db: -12.5, muted: true });
    expect(sanitizeMonitor({ db: null })).toEqual({ db: null });
    expect(sanitizeMonitor({ db: 3, muted: 'yes' })).toEqual({});
    expect(sanitizeMonitor({ db: -61 })).toEqual({});
    expect(sanitizeMonitor({ db: Number.NaN })).toEqual({});
  });
});

describe('the monitor volume sync', () => {
  it('tells the player the saved volume at start, then every change, newest first', async () => {
    vi.mocked(loadMonitor).mockResolvedValue({ db: -12, muted: false });
    let release!: () => void;
    vi.mocked(playerVolume).mockImplementationOnce(
      () => new Promise<void>((resolve) => (release = resolve)),
    );
    const stop = startMonitorSync();
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenCalledWith(-12));
    expect(useMonitor.getState()).toMatchObject({ db: -12, hydrated: true });
    // A drag while the first command is on its way: only the last value follows it.
    useMonitor.getState().setDb(-11);
    useMonitor.getState().setDb(-10);
    useMonitor.getState().setDb(-9.5);
    expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(1);
    release();
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(2));
    expect(vi.mocked(playerVolume)).toHaveBeenLastCalledWith(-9.5);
    useMonitor.getState().toggleMute();
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenLastCalledWith(null));
    stop();
  });

  it('asks a refusing player again only on the next change, never in a loop', async () => {
    vi.mocked(playerVolume).mockRejectedValue({ message: 'no output device' });
    const stop = startMonitorSync();
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(1);
    useMonitor.getState().step(-1);
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(2));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(vi.mocked(playerVolume)).toHaveBeenCalledTimes(2);
    expect(vi.mocked(playerVolume)).toHaveBeenLastCalledWith(-1);
    stop();
  });

  it('saves once after a drag, and never writes back what it read', async () => {
    vi.useFakeTimers();
    const stop = startMonitorSync();
    await vi.advanceTimersByTimeAsync(0);
    expect(useMonitor.getState().hydrated).toBe(true);
    await vi.advanceTimersByTimeAsync(MONITOR_SAVE_DELAY_MS * 2);
    expect(vi.mocked(saveMonitor)).not.toHaveBeenCalled();
    for (const db of [-1, -2, -3, -4]) useMonitor.getState().setDb(db);
    await vi.advanceTimersByTimeAsync(MONITOR_SAVE_DELAY_MS + 10);
    expect(vi.mocked(saveMonitor).mock.calls).toEqual([[{ db: -4, muted: false }]]);
    stop();
  });

  it('defaults to 0 dB when nothing was saved', async () => {
    const stop = startMonitorSync();
    await vi.waitFor(() => expect(vi.mocked(playerVolume)).toHaveBeenCalledWith(0));
    expect(useMonitor.getState()).toMatchObject({ ...DEFAULT_MONITOR, hydrated: true });
    stop();
  });
});
