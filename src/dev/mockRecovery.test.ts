import { mockRecovery, recoveryDemoOf } from './mockRecovery';

describe('the stand-in recovery', () => {
  it('is picked by the URL', () => {
    expect(recoveryDemoOf('?demo=recovery')).toBe('pending');
    expect(recoveryDemoOf('?demo=recovery-failed')).toBe('failed');
    expect(recoveryDemoOf('?demo=recovery-finished')).toBe('finished');
    expect(recoveryDemoOf('?demo=1')).toBe('none');
    expect(recoveryDemoOf('')).toBe('none');
  });

  it('runs a moment, refuses Forget meanwhile, then forgets as the shell does', () => {
    let t = 0;
    const r = mockRecovery('pending', () => t);
    expect(r.status()).toEqual({ state: 'running' });
    expect(() => r.forget('6ac5b4f4-75821-0')).toThrow();
    t = 10_000;
    const before = r.status();
    expect(before.state === 'finished' && before.pending).toHaveLength(2);
    const result = r.forget('6ac5b4f4-75821-0');
    expect(result.backup).toMatch(/SoundCheck Backups/);
    expect(result.status.state === 'finished' && result.status.pending).toHaveLength(1);
    expect(() => r.forget('6ac5b4f4-75821-0')).toThrow();
  });
});
