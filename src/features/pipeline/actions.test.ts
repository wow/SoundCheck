import { act } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import { analysed, analysis, doneRow, entry, plan } from '@/test/fixtures';
import type { JobEvent, SessionSnapshot } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { useSettings } from '@/state/settings';

const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();

/** The pipeline module holds one actor: each test takes a fresh copy of the module. */
async function fresh() {
  vi.resetModules();
  // The store and the machine come back with the module; keep the singletons the test reads.
  const lib = await import('@/state/library');
  const actions = await import('./actions');
  return { useLibrary: lib.useLibrary, ...actions };
}

const request = {
  fileIds: [1],
  settings: {
    batchMode: 'prepare' as const,
    place: 'inPlace' as const,
    depth: null,
    gridOnly: false,
    tbpm: true,
    xml: true,
    leadMs: 5,
  },
  outDir: null,
  confirmed: null,
};

const snapshot = (overrides: Partial<SessionSnapshot> = {}): SessionSnapshot => ({
  revision: 0,
  rows: [],
  runningJobs: [],
  artefacts: null,
  artefactsFailed: null,
  lastExport: null,
  ...overrides,
});

beforeEach(() => {
  useLibrary.setState(initialLibrary, true);
  useSettings.setState(initialSettings, true);
});

describe('starting a job', () => {
  it('a refused analysis keeps the typed error, not [object Object]', async () => {
    const { pipeline, useLibrary: lib } = await fresh();
    mockIPC((cmd) => {
      if (cmd === 'analyze') throw { kind: 'busy', message: 'being exported', fileId: 1 };
      return undefined;
    });
    lib.getState().add([entry(1)]);
    pipeline().send({ type: 'ANALYSE', fileIds: [1] });
    await vi.waitFor(() => expect(lib.getState().aborted).not.toBeNull());
    expect(lib.getState().aborted).toEqual({ kind: 'busy', message: 'being exported', fileId: 1 });
    expect(pipeline().getSnapshot().value).toBe('idle');
    expect(lib.getState().rows[1]?.state).toBe('cancelled');
  });

  it('a refused export says why and leaves its rows as they were', async () => {
    const { pipeline, useLibrary: lib } = await fresh();
    mockIPC((cmd) => {
      if (cmd === 'export') throw { kind: 'recoveryRunning', message: 'running', fileId: null };
      return undefined;
    });
    lib.getState().add([entry(1)]);
    lib.getState().applyEvent(analysed(1));
    pipeline().send({ type: 'EXPORT', job: { request, planned: [] } });
    expect(lib.getState().rows[1]?.state).toBe('exportQueued');
    await vi.waitFor(() => expect(lib.getState().aborted?.kind).toBe('recoveryRunning'));
    expect(lib.getState().abortedKind).toBe('export');
    expect(lib.getState().rows[1]?.state).toBe('analysed');
    expect(pipeline().getSnapshot().value).toBe('idle');
  });

  it('runs an export to its end over the channel', async () => {
    const { pipeline, useLibrary: lib } = await fresh();
    let send: (e: JobEvent) => void = () => {};
    mockIPC((cmd, args) => {
      if (cmd === 'export') {
        const channel = (args as { onEvent: { onmessage: (e: JobEvent) => void } }).onEvent;
        send = (e) => channel.onmessage(e);
        return 11;
      }
      return undefined;
    });
    lib.getState().add([entry(1)]);
    lib.getState().applyEvent(analysed(1));
    pipeline().send({ type: 'EXPORT', job: { request, planned: [] } });
    await vi.waitFor(() => expect(pipeline().getSnapshot().context.jobId).toBe(11));
    send({ type: 'exportDone', jobId: 11, fileId: 1, row: doneRow(), inPlace: false, update: null });
    send({ type: 'finished', jobId: 11, cancelled: false });
    expect(lib.getState().rows[1]?.state).toBe('done');
    expect(pipeline().getSnapshot().value).toBe('idle');
  });

  it('an in-place row with no update is analysed again after the export', async () => {
    const { pipeline, useLibrary: lib } = await fresh();
    const started: string[] = [];
    let send: (e: JobEvent) => void = () => {};
    mockIPC((cmd, args) => {
      started.push(cmd);
      if (cmd === 'export') {
        const channel = (args as { onEvent: { onmessage: (e: JobEvent) => void } }).onEvent;
        send = (e) => channel.onmessage(e);
        return 11;
      }
      if (cmd === 'analyze') return 12;
      return undefined;
    });
    lib.getState().add([entry(1)]);
    lib.getState().applyEvent(analysed(1));
    pipeline().send({ type: 'EXPORT', job: { request, planned: [] } });
    await vi.waitFor(() => expect(pipeline().getSnapshot().context.jobId).toBe(11));
    send({ type: 'exportDone', jobId: 11, fileId: 1, row: doneRow(), inPlace: true, update: null });
    expect(started).not.toContain('analyze');
    send({ type: 'finished', jobId: 11, cancelled: false });
    await vi.waitFor(() => expect(started).toContain('analyze'));
    expect(pipeline().getSnapshot().value).toBe('analysing');
  });
});

describe('a reload during an export', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('reads the session until the job has ended, then finds the links and starts what waited', async () => {
    const { pipeline, useLibrary: lib, WIND_DOWN_POLL_MS } = await fresh();
    const answers = [
      snapshot({
        runningJobs: [9],
        rows: [{ entry: entry(1), row: null, plan: null, export: null, revertible: false, exportStale: false }],
      }),
      snapshot({
        runningJobs: [],
        artefacts: { folder: '/x', xml: '/x/a.xml', report: '/x/a.csv', listed: 1 },
        rows: [{ entry: entry(1), row: analysis(), plan: plan(), export: doneRow(), revertible: true, exportStale: false }],
      }),
    ];
    const reads: number[] = [];
    mockIPC((cmd) => {
      if (cmd === 'restore_session') {
        reads.push(1);
        return answers.shift();
      }
      return undefined;
    });
    pipeline().send({ type: 'WIND_DOWN' });
    expect(pipeline().getSnapshot().value).toBe('windingDown');

    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(reads).toHaveLength(1);
    expect(pipeline().getSnapshot().value).toBe('windingDown');
    expect(lib.getState().locked).toBe('all');

    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(reads).toHaveLength(2);
    expect(pipeline().getSnapshot().value).toBe('idle');
    expect(lib.getState().locked).toBeNull();
    expect(lib.getState().artefacts?.xml).toBe('/x/a.xml');
    expect(lib.getState().rows[1]?.revertible).toBe(true);
  });

  it('analyses a file written in place during the reload, once the job has ended', async () => {
    const { pipeline, useLibrary: lib, WIND_DOWN_POLL_MS } = await fresh();
    const calls: string[] = [];
    const answers = [
      snapshot({
        rows: [{ entry: entry(1), row: null, plan: null, export: doneRow(), revertible: true, exportStale: false }],
      }),
    ];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === 'restore_session') return answers.shift();
      return cmd === 'analyze' ? 3 : undefined;
    });
    pipeline().send({ type: 'WIND_DOWN' });
    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(calls).toContain('analyze');
    expect(pipeline().getSnapshot().value).toBe('analysing');
    expect(lib.getState().rows[1]).toMatchObject({ state: 'queued', revertible: true });
    expect(lib.getState().rows[1]?.export?.stage).toBe('done');
  });

  it('says stopped by the reload only of the export the reload stopped', async () => {
    const summary = { jobId: 9, written: 1, xmlOnly: 0, skipped: 0, failed: 0, cancelled: true, seconds: 3 };
    const { pipeline, useLibrary: lib, WIND_DOWN_POLL_MS } = await fresh();
    // A reload during an analysis (job 5), with an older export (job 9) on record.
    const answers = [snapshot({ runningJobs: [], lastExport: summary })];
    mockIPC((cmd) => (cmd === 'restore_session' ? answers.shift() : undefined));
    lib.getState().restore(snapshot({ runningJobs: [5], lastExport: summary }), [70, 180]);
    pipeline().send({ type: 'WIND_DOWN' });
    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(lib.getState().lastExport?.stoppedByReload).toBeFalsy();

    // The export itself.
    const again = [snapshot({ runningJobs: [], lastExport: summary })];
    mockIPC((cmd) => (cmd === 'restore_session' ? again.shift() : undefined));
    lib.getState().restore(snapshot({ runningJobs: [9] }), [70, 180]);
    pipeline().send({ type: 'WIND_DOWN' });
    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(lib.getState().lastExport?.stoppedByReload).toBe(true);
  });

  it('gives up after a minute and says to restart', async () => {
    const { pipeline, useLibrary: lib, WIND_DOWN_LIMIT_MS } = await fresh();
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'restore_session') return undefined;
      reads += 1;
      return snapshot({ runningJobs: [9] });
    });
    pipeline().send({ type: 'WIND_DOWN' });
    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_LIMIT_MS - 1000));
    expect(pipeline().getSnapshot().value).toBe('windingDown');
    expect(lib.getState().notice).toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(pipeline().getSnapshot().value).toBe('idle');
    expect(lib.getState().notice).toMatch(/Restart SoundCheck/);
    const at = reads;
    await act(async () => vi.advanceTimersByTimeAsync(5000));
    expect(reads).toBe(at);
  });

  it('does not wait for ever when the session cannot be read', async () => {
    const { pipeline, useLibrary: lib, WIND_DOWN_POLL_MS } = await fresh();
    mockIPC(() => {
      throw { kind: 'internal', message: 'gone', fileId: null };
    });
    pipeline().send({ type: 'WIND_DOWN' });
    await act(async () => vi.advanceTimersByTimeAsync(WIND_DOWN_POLL_MS));
    expect(pipeline().getSnapshot().value).toBe('idle');
    expect(lib.getState().locked).toBeNull();
  });
});

