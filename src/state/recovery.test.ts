import { act } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { RecoveryStatus } from '@/lib/ipc';
import { resetRecovery, RETRY_MS, startRecoveryWatch, useRecovery } from './recovery';

const pending: RecoveryStatus = {
  state: 'finished',
  recovered: [],
  pending: [{ txn: '6ac5b4f4-1-0', path: '/Volumes/S/a.wav', reason: 'gone', toFolder: true }],
};

const advance = (ms: number) => act(async () => vi.advanceTimersByTimeAsync(ms));
const allRetries = RETRY_MS.reduce((a, b) => a + b, 0);

beforeEach(() => {
  vi.useFakeTimers();
  resetRecovery();
});
afterEach(() => vi.useRealTimers());

describe('the recovery watch', () => {
  it('keeps the whole status from the engine, and stops reading once it ended', async () => {
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'recovery_status') return undefined;
      reads += 1;
      return pending;
    });
    const stop = startRecoveryWatch();
    await advance(5000);
    expect(useRecovery.getState().status).toEqual(pending);
    expect(useRecovery.getState().running).toBe(false);
    expect(reads).toBe(1);
    stop();
  });

  it('reads again after a failed read, with a short wait, and carries on', async () => {
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'recovery_status') return undefined;
      reads += 1;
      if (reads <= 2) throw new Error('not ready');
      return pending;
    });
    const stop = startRecoveryWatch();
    await advance(allRetries);
    expect(reads).toBe(3);
    expect(useRecovery.getState().status).toEqual(pending);
    stop();
  });

  it('says it could not read the check after the last retry, rather than nothing', async () => {
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'recovery_status') return undefined;
      reads += 1;
      throw new Error('the window is closing');
    });
    const stop = startRecoveryWatch();
    await advance(allRetries + 1000);
    expect(reads).toBe(RETRY_MS.length + 1);
    expect(useRecovery.getState().status).toEqual({
      state: 'failed',
      message: 'what it found could not be read: the window is closing',
    });
    expect(useRecovery.getState().running).toBe(false);
    stop();
  });

  it('reads nothing more once stopped', async () => {
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'recovery_status') return undefined;
      reads += 1;
      throw new Error('not ready');
    });
    const stop = startRecoveryWatch();
    await advance(0);
    stop();
    await advance(allRetries + 1000);
    expect(reads).toBe(1);
    expect(useRecovery.getState().status).toBeNull();
  });
});
