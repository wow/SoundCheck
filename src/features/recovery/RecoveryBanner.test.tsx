import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ForgetResult, PendingChange, RecoveryStatus } from '@/lib/ipc';
import { resetRecovery, useRecovery } from '@/state/recovery';
import { RecoveryBanner, RUNNING_SHOW_AFTER_MS } from './RecoveryBanner';

/** A copy to a stick that is not plugged in. */
const stick: PendingChange = {
  txn: '6ac5b4f4-75821-0',
  path: '/Volumes/STICK/Crate/Night Drive.wav',
  reason: '/Volumes/STICK/Crate is not reachable (volume not mounted?)',
  toFolder: true,
};
/** A change in place, still running in another window. */
const busy: PendingChange = {
  txn: '6ac5b4f4-75821-1',
  path: '/Music/Crate/Big Room.aiff',
  reason: 'running',
  toFolder: false,
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

function show(status: RecoveryStatus | null) {
  useRecovery.setState({ status, running: status?.state === 'running' });
  return render(<RecoveryBanner />);
}

const live = () => screen.getByTestId('recovery-live');

/** The drawn lines, leaving out the live region that repeats them for screen readers. */
const DRAWN = { ignore: '[data-testid="recovery-live"], script, style' };

/** Presses Forget on `name`'s row, then Forget in the step that asks. */
async function forget(name: string) {
  await userEvent.click(screen.getByRole('button', { name: `Forget the change to ${name}` }));
  await userEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Forget' }));
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
      expect(screen.getByText('Finishing changes from the last session…', DRAWN)).toBeInTheDocument();
      expect(screen.getByText(/Export, Revert and the rekordbox XML wait/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps one live region from the start and changes only its text', async () => {
    vi.useFakeTimers();
    try {
      show(null);
      const region = live();
      expect(region).toHaveAttribute('role', 'status');
      expect(region).toHaveAttribute('aria-live', 'polite');
      expect(region).toBeEmptyDOMElement();
      act(() => useRecovery.setState({ status: { state: 'running' }, running: true }));
      await act(async () => vi.advanceTimersByTimeAsync(RUNNING_SHOW_AFTER_MS));
      expect(live()).toBe(region);
      expect(region).toHaveTextContent('Finishing changes from the last session…');
      act(() => useRecovery.setState({ status: finished([stick], 1), running: false }));
      expect(live()).toBe(region);
      expect(region).toHaveTextContent(
        '1 change from the last session could not be finished 1 change from the last session was finished.',
      );
      // The only status on the page: the lines themselves are not live regions of their own.
      expect(screen.getAllByRole('status')).toEqual([region]);
    } finally {
      vi.useRealTimers();
    }
  });

  it('lists each change it could not finish: the file, why, and Forget, in a region', () => {
    show(finished([stick, busy]));
    const banner = screen.getByRole('region', { name: 'Changes from the last session' });
    expect(banner).toHaveAttribute('role', 'region');
    expect(
      within(banner).getByText('2 changes from the last session could not be finished'),
    ).toBeInTheDocument();
    expect(within(banner).getByText('Night Drive.wav')).toHaveAttribute('title', stick.path);
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

  it('says what Forget means for a change in place, and Cancel forgets nothing', async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
    });
    show(finished([stick, busy]));
    await userEvent.click(
      screen.getByRole('button', { name: 'Forget the change to Big Room.aiff' }),
    );
    const dialog = screen.getByRole('dialog', { name: 'Forget the change to Big Room.aiff?' });
    expect(within(dialog).getByText(/stops trying to finish this change/)).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        /^The file is left as it is now, which may be the original or the new version/,
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(/^Nothing is deleted: a backup of the original/),
    ).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls).not.toContain('forget_pending');
  });

  it('says what Forget means for a copy to a folder: no copy yet, an empty file, or the copy', async () => {
    show(finished([stick]));
    await userEvent.click(
      screen.getByRole('button', { name: 'Forget the change to Night Drive.wav' }),
    );
    const dialog = screen.getByRole('dialog');
    expect(
      within(dialog).getByText(
        'The copy is left as it is now: depending on how far the change got, there is no copy yet, an empty file holding its name, or the new copy. The original was not changed.',
      ),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/backup of the original/)).toBeNull();
  });

  it('forgets on confirm, removes the row, and lists each kept file by its path', async () => {
    const asked: unknown[] = [];
    const temp = '/Volumes/STICK/Crate/.Night Drive.wav.soundcheck-tmp-42-7f';
    const answer: ForgetResult = {
      path: stick.path,
      backup: null,
      kept: [temp, stick.path],
      status: finished([busy]),
    };
    mockIPC((cmd, args) => {
      if (cmd !== 'forget_pending') return undefined;
      asked.push(args);
      return answer;
    });
    show(finished([stick, busy]));
    await forget('Night Drive.wav');
    expect(asked).toEqual([{ txn: stick.txn }]);
    expect(
      await screen.findByText('1 change from the last session could not be finished', DRAWN),
    ).toBeInTheDocument();
    expect(screen.queryByText('Night Drive.wav')).toBeNull();
    expect(
      screen.getByText('The change to Night Drive.wav is forgotten. Nothing was deleted.'),
    ).toBeInTheDocument();
    const kept = within(screen.getByRole('list', { name: 'Kept' })).getAllByRole('listitem');
    expect(kept).toHaveLength(2);
    expect(kept[0]).toHaveTextContent(
      `${temp}: a hidden copy the change was writing, which may be complete`,
    );
    expect(kept[1]).toHaveTextContent(
      `${stick.path}: may be an empty file holding the copy's name; remove it before exporting`,
    );
    expect(live()).toHaveTextContent(/^The change to Night Drive\.wav is forgotten\./);
  });

  it('names the backup of a change in place among what it kept', async () => {
    const backup = '/Users/me/Music/SoundCheck Backups/2026-11-05/Big Room.aiff';
    mockIPC((cmd) =>
      cmd === 'forget_pending'
        ? { path: busy.path, backup, kept: [backup], status: finished([]) }
        : undefined,
    );
    show(finished([busy]));
    await forget('Big Room.aiff');
    expect(await screen.findByText(`${backup}`)).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'Kept' })).toHaveTextContent(
      `${backup}: the backup of the original`,
    );
  });

  it('drops a row whose change ended meanwhile, when Forget is refused for it', async () => {
    mockIPC((cmd) => {
      if (cmd === 'forget_pending') {
        throw {
          kind: 'invalidArgument',
          message:
            'invalid argument: change 6ac5b4f4-75821-0 is already recovered; only an unfinished change can be forgotten',
          fileId: null,
        };
      }
      return cmd === 'recovery_status' ? finished([busy]) : undefined;
    });
    show(finished([stick, busy]));
    await forget('Night Drive.wav');
    expect(
      await screen.findByText('1 change from the last session could not be finished', DRAWN),
    ).toBeInTheDocument();
    expect(screen.queryByText('Night Drive.wav')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows a refusal for a change still pending in plain words, without the class name', async () => {
    mockIPC((cmd) => {
      if (cmd === 'forget_pending') {
        throw {
          kind: 'invalidArgument',
          message:
            'invalid argument: change 6ac5b4f4-75821-1 is running; it can only be forgotten once it has stopped',
          fileId: null,
        };
      }
      return cmd === 'recovery_status' ? finished([busy]) : undefined;
    });
    show(finished([busy]));
    await forget('Big Room.aiff');
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(
      'Change 6ac5b4f4-75821-1 is running; it can only be forgotten once it has stopped. Try again once it has stopped; if it is still refused, restart SoundCheck to check it again.',
    );
    expect(alert).not.toHaveTextContent(/invalid argument/i);
    expect(screen.getByText('Big Room.aiff')).toBeInTheDocument();
  });

  it('with only finished changes, says so in one line that can be closed', async () => {
    show(finished([], 2));
    expect(screen.getByText('2 changes from the last session were finished.', DRAWN)).toBeInTheDocument();
    expect(live()).toHaveTextContent('2 changes from the last session were finished.');
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByText(/were finished/, DRAWN)).toBeNull();
    expect(live()).toBeEmptyDOMElement();
  });

  it('says an undone change left its file as it was', () => {
    show({
      state: 'finished',
      pending: [],
      recovered: [{ txn: '6ac5b4f4-1-0', path: '/Music/a.wav', outcome: 'rolledBack' }],
    });
    expect(
      screen.getByText('1 change from the last session was undone; its file is as it was.', DRAWN),
    ).toBeInTheDocument();
  });

  it('when the check could not run: what, why, what to do', () => {
    show({ state: 'failed', message: 'recovery stopped before it finished' });
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('The changes from the last session could not be checked');
    expect(alert).toHaveTextContent('Recovery stopped before it finished.');
    expect(alert).toHaveTextContent(/Restart SoundCheck to check again/);
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
});
