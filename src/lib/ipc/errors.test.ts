import { errorCopy, ipcErrorOf, rowErrorCopy } from './errors';

describe('rejected commands', () => {
  it('keeps the typed error Rust sent, with its kind', () => {
    const sent = { kind: 'busy', message: 'being exported', fileId: 3 };
    expect(ipcErrorOf(sent)).toEqual(sent);
  });

  it('never turns anything into [object Object]', () => {
    expect(ipcErrorOf({ unexpected: true }).message).toBe('The command failed.');
    expect(ipcErrorOf(new Error('boom'))).toEqual({ kind: 'internal', message: 'boom', fileId: null });
    expect(ipcErrorOf('plain text').message).toBe('plain text');
    expect(ipcErrorOf(undefined).message).not.toContain('[object');
  });

  it('words the timing refusals so the user can act on them', () => {
    const busy = errorCopy({ kind: 'busy', message: 'raw', fileId: null });
    expect(busy).toMatch(/another job/i);
    expect(busy).not.toContain('raw');
    const recovery = errorCopy({ kind: 'recoveryRunning', message: 'raw', fileId: null });
    expect(recovery).toMatch(/crash interrupted/);
    expect(recovery).not.toContain('raw');
    expect(errorCopy({ kind: 'corrupt', message: 'truncated', fileId: 1 })).toBe('Truncated');
    expect(errorCopy({ kind: 'settingsChanged', message: 'raw', fileId: null })).toMatch(/Press Export again/);
  });

  it('on a file\'s own row, says it of this file rather than its long path', () => {
    const error = { kind: 'fileChanged' as const, message: '/music/a.wav changed since you confirmed the export', fileId: 1 };
    expect(rowErrorCopy(error, '/music/a.wav')).toBe('This file changed since you confirmed the export');
    expect(rowErrorCopy(error, '/other.wav')).toBe('/music/a.wav changed since you confirmed the export');
  });
});
