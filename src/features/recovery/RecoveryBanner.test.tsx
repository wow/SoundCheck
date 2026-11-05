import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ForgetResult, PendingChange, RecoveryStatus } from '@/lib/ipc';
import { resetRecovery, startRecoveryWatch, useRecovery } from '@/state/recovery';
import { RecoveryBanner, RUNNING_SHOW_AFTER_MS } from './RecoveryBanner';

const stick: PendingChange = {
  txn: '6ac5b4f4-75821-0',
  path: '/Volumes/STICK/Crate/Night Drive.wav',
  reason: '/Volumes/STICK/Crate is not reachable (volume not mounted?)',
};
const busy: PendingChange = {
  txn: '6ac5b4f4-75821-1',
  path: '/Music/Crate/Big Room.aiff',
  reason: 'running',
};

function finished(pending: PendingChange[], recovered: number = 0): RecoveryStatus {
  return {
    state: 'finished',
    pending,
    recovered: Array.from({ length: recovered }, (_, i) => ({
      txn: `6ac5b4f4-1-${i}`,
      path: `/Music/Crate/Done ${i}.wav`,
      outcome: 'completed' as const,
    })),
  };
}

function show(status: RecoveryStatus) {
  useRecovery.setState({ status, running: status.state === 'running' });
  return render(<RecoveryBanner />);
}

beforeEach(() => {
  resetRecovery();
  mockIPC(() => undefined);
});

describe('the recovery banner', () => {
  it('while the check runs, says it is finishing the last session, once it has run a moment', async () => {
    vi.useFakeTimers();
    try {
      show({ state: 'running' });
      expect(screen.queryByText(/Finishing changes/)).toBeNull();
      await act(async () => vi.advanceTimersByTimeAsync(RUNNING_SHOW_AFTER_MS));
      expect(screen.getByRole('status')).toHaveTextContent(
        'Finishing changes from the last session…',
      );
      expect(screen.getByRole('status')).toHaveTextContent(
        /Export, Revert and the rekordbox XML wait/,
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('lists each change it could not finish: the file, why, and Forget', () => {
    show(finished([stick, busy]));
    const banner = screen.getByRole('region', { name: 'Changes from the last session' });
    expect(
      within(banner).getByText('2 changes from the last session could not be finished'),
    ).toBeInTheDocument();
    const name = within(banner).getByText('Night Drive.wav');
    expect(name).toHaveAttribute('title', stick.path);
    expect(
      within(banner).getByText('/Volumes/STICK/Crate is not reachable (volume not mounted?)'),
    ).toBeInTheDocument();
    expect(
      within(banner).getByText('It is still running in another SoundCheck window'),
    ).toBeInTheDocument();
    expect(within(banner).getAllByRole('button', { name: /^Forget the change to/ })).toHaveLength(
      2,
    );
  });

  it('says what Forget means before it forgets, and Cancel forgets nothing', async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
    });
    show(finished([stick, busy]));
    await userEvent.click(
      screen.getByRole('button', { name: 'Forget the change to Night Drive.wav' }),
    );
    const dialog = screen.getByRole('dialog', { name: 'Forget the change to Night Drive.wav?' });
    expect(within(dialog).getByText(/stops trying to finish this change/)).toBeInTheDocument();
    expect(within(dialog).getByText(/The file is left as it is now/)).toBeInTheDocument();
    expect(
      within(dialog).getByText(/^Nothing is deleted: a backup of the original/),
    ).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls).not.toContain('forget_pending');
  });

  it('forgets on confirm, removes the row, and says where the backup stays', async () => {
    const asked: unknown[] = [];
    const answer: ForgetResult = {
      path: stick.path,
      backup: '/Users/me/Music/SoundCheck Backups/2026-11-05/STICK/Crate/Night Drive.wav',
      kept: ['/Users/me/Music/SoundCheck Backups/2026-11-05/STICK/Crate/Night Drive.wav'],
      status: finished([busy]),
    };
    mockIPC((cmd, args) => {
      if (cmd === 'forget_pending') {
        asked.push(args);
        return answer;
      }
      return undefined;
    });
    show(finished([stick, busy]));
    await userEvent.click(
      screen.getByRole('button', { name: 'Forget the change to Night Drive.wav' }),
    );
    await userEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Forget' }),
    );
    expect(asked).toEqual([{ txn: stick.txn }]);
    expect(
      await screen.findByText('1 change from the last session could not be finished'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Night Drive.wav')).toBeNull();
    expect(
      screen.getByText(
        /^The change to Night Drive\.wav is forgotten\. Nothing was deleted\. The backup/,
      ),
    ).toBeInTheDocument();
  });

  it('shows a refused Forget on its row, with what to do', async () => {
    mockIPC((cmd) => {
      if (cmd === 'forget_pending') {
        throw {
          kind: 'invalidArgument',
          message:
            'change 6ac5b4f4-75821-1 is running; it can only be forgotten once it has stopped',
          fileId: null,
        };
      }
      return undefined;
    });
    show(finished([busy]));
    await userEvent.click(
      screen.getByRole('button', { name: 'Forget the change to Big Room.aiff' }),
    );
    await userEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Forget' }),
    );
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(
      /^Change 6ac5b4f4-75821-1 is running; it can only be forgotten once it has stopped\. Try again/,
    );
    expect(screen.getByText('Big Room.aiff')).toBeInTheDocument();
  });

  it('with only finished changes, says so in one line that can be closed', async () => {
    show(finished([], 2));
    expect(screen.getByRole('status')).toHaveTextContent(
      '2 changes from the last session were finished.',
    );
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText(/were finished/)).toBeNull();
  });

  it('says an undone change left its file as it was', () => {
    show({
      state: 'finished',
      pending: [],
      recovered: [{ txn: '6ac5b4f4-1-0', path: '/Music/a.wav', outcome: 'rolledBack' }],
    });
    expect(screen.getByRole('status')).toHaveTextContent(
      '1 change from the last session was undone; its file is as it was.',
    );
  });

  it('when the check could not run: what, why, what to do', () => {
    show({ state: 'failed', message: 'recovery stopped before it finished' });
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('The changes from the last session could not be checked');
    expect(alert).toHaveTextContent('Recovery stopped before it finished.');
    expect(alert).toHaveTextContent(/Nothing was changed\. Restart SoundCheck to check again/);
  });

  it('says nothing when it skipped, or found nothing', () => {
    const { unmount } = show({
      state: 'skipped',
      reason: 'there is no backup folder (no home folder found)',
    });
    expect(document.body.textContent).toBe('');
    unmount();
    show(finished([]));
    expect(document.body.textContent).toBe('');
  });

  it('keeps the whole status from the engine, and stops reading once it ended', async () => {
    let reads = 0;
    mockIPC((cmd) => {
      if (cmd !== 'recovery_status') return undefined;
      reads += 1;
      return finished([stick]);
    });
    const stop = startRecoveryWatch();
    await vi.waitFor(() => expect(useRecovery.getState().status).toEqual(finished([stick])));
    expect(useRecovery.getState().running).toBe(false);
    expect(reads).toBe(1);
    stop();
  });
});
