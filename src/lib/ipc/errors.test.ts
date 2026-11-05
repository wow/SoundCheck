import { errorCopy, ipcErrorOf, rowErrorCopy } from './errors';

describe('rejected commands', () => {
  it('keeps the typed error Rust sent, with its kind', () => {
    const sent = { kind: 'busy', message: 'being exported', fileId: 3 };
    expect(ipcErrorOf(sent)).toEqual(sent);
  });

  it('never turns anything into [object Object]', () => {
    expect(ipcErrorOf({ unexpected: true }).message).toBe('The command failed.');
    expect(ipcErrorOf(new Error('boom'))).toEqual({
      kind: 'internal',
      message: 'boom',
      fileId: null,
    });
    expect(ipcErrorOf('plain text').message).toBe('plain text');
    expect(ipcErrorOf(undefined).message).not.toContain('[object');
  });

  it('words the timing refusals so the user can act on them, after the engine says why', () => {
    const busy = errorCopy({
      kind: 'busy',
      message: '/m/a.wav is being exported; try again when that has finished',
      fileId: 1,
    });
    expect(busy).toBe('/m/a.wav is being exported. Wait for it to finish, then try again.');
    const recovery = errorCopy({
      kind: 'recoveryRunning',
      message: 'the check for changes a crash interrupted is still running; nothing was done',
      fileId: null,
    });
    expect(recovery).toBe(
      'The check for changes a crash interrupted is still running. Nothing was changed. Try again in a moment.',
    );
    expect(errorCopy({ kind: 'corrupt', message: 'truncated', fileId: 1 })).toMatch(
      /^Truncated\. /,
    );
    expect(
      errorCopy({
        kind: 'settingsChanged',
        message: 'the loudness settings changed after this plan; nothing was done',
        fileId: null,
      }),
    ).toBe(
      'The loudness settings changed after this plan. Nothing was written. Press Export again to see the new numbers.',
    );
  });

  it("on a file's own row, says it of this file rather than its long path", () => {
    const error = {
      kind: 'fileChanged' as const,
      message: '/music/a.wav changed since you confirmed the export',
      fileId: 1,
    };
    expect(rowErrorCopy(error, '/music/a.wav')).toBe(
      'This file changed since you confirmed the export. Press Analyse to read it again.',
    );
    expect(rowErrorCopy(error, '/other.wav')).toMatch(/^\/music\/a\.wav changed since/);
  });
});
