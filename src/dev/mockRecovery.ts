/**
 * The start-up recovery of the browser stand-in, picked by the URL so each banner can be looked
 * at: `?demo=recovery` (the check runs for a moment, then two changes are left pending and one is
 * finished), `?demo=recovery-failed`, `?demo=recovery-finished`; anything else finds nothing.
 * Reached through `mockBackend.ts` only.
 */
import type { ForgetResult, RecoveryStatus } from '@/lib/ipc';

export type RecoveryDemo = 'none' | 'pending' | 'failed' | 'finished';

/** How long the pending demo's check runs before it answers. */
const RUNNING_MS = 2500;
const BACKUPS = '/Users/mock/Music/SoundCheck Backups/2026-11-04';

/** The demo the page's URL asks for. */
export function recoveryDemoOf(search: string): RecoveryDemo {
  switch (new URLSearchParams(search).get('demo')) {
    case 'recovery':
      return 'pending';
    case 'recovery-failed':
      return 'failed';
    case 'recovery-finished':
      return 'finished';
    default:
      return 'none';
  }
}

const PENDING = [
  {
    txn: '6ac5b4f4-75821-0',
    path: '/Volumes/GIG STICK/Crates/Friday/Mock Artist A - Night Drive (Extended Mix).flac',
    reason: '/Volumes/GIG STICK/Crates/Friday is not reachable (volume not mounted?)',
    toFolder: true,
  },
  {
    txn: '6ac5b4f4-75821-1',
    path: '/Users/mock/Music/Crate/Mock Artist J - Big Room.wav',
    reason: 'cleanup incomplete',
    toFolder: false,
  },
];

function initial(demo: RecoveryDemo): RecoveryStatus {
  switch (demo) {
    case 'pending':
      return {
        state: 'finished',
        recovered: [
          {
            txn: '6ac5b4f4-75820-3',
            path: '/Users/mock/Music/Crate/Mock Artist C - Quiet Classic.aiff',
            outcome: 'completed',
          },
        ],
        pending: PENDING,
      };
    case 'failed':
      return {
        state: 'failed',
        message: 'I/O error on /Users/mock/Music/SoundCheck Backups/journal.jsonl: Permission denied (os error 13)',
      };
    case 'finished':
      return {
        state: 'finished',
        recovered: [
          {
            txn: '6ac5b4f4-75820-1',
            path: '/Users/mock/Music/Crate/Mock Artist A - Night Drive.flac',
            outcome: 'completed',
          },
          {
            txn: '6ac5b4f4-75820-2',
            path: '/Users/mock/Music/Crate/Mock Artist J - Big Room.wav',
            outcome: 'rolledBack',
          },
        ],
        pending: [],
      };
    case 'none':
      return { state: 'finished', recovered: [], pending: [] };
  }
}

/** A recovery that answers `recovery_status` and `forget_pending` as the shell does. */
export function mockRecovery(demo: RecoveryDemo, now: () => number = Date.now) {
  const started = now();
  let status = initial(demo);
  const running = () => demo === 'pending' && now() - started < RUNNING_MS;
  return {
    status(): RecoveryStatus {
      return running() ? { state: 'running' } : status;
    },
    forget(txn: string): ForgetResult {
      if (running()) {
        throw {
          kind: 'recoveryRunning',
          message: 'the check for changes a crash interrupted is still running; nothing was done',
          fileId: null,
        };
      }
      if (status.state !== 'finished' || !status.pending.some((p) => p.txn === txn)) {
        throw {
          kind: 'invalidArgument',
          message: `change ${txn} is not left pending by recovery`,
          fileId: null,
        };
      }
      const change = status.pending.find((p) => p.txn === txn);
      status = { ...status, pending: status.pending.filter((p) => p.txn !== txn) };
      const path = change?.path ?? '';
      const slash = path.lastIndexOf('/');
      const name = path.slice(slash + 1);
      // As the shell answers: a copy keeps its hidden temp file and the empty file holding its
      // name; a change in place keeps the backup of the original.
      if (change?.toFolder) {
        const temp = `${path.slice(0, slash)}/.${name}.soundcheck-tmp-4242-7f3a`;
        return { path, backup: null, kept: [temp, path], status };
      }
      const backup = `${BACKUPS}/${name}`;
      return { path, backup, kept: [backup], status };
    },
  };
}
