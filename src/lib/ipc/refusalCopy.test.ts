import type { IpcErrorKind } from './generated/IpcErrorKind';
import { refusal, SURFACES, surfacesOf, todoFor } from './refusalCopy';

/**
 * Every class the engine sends, by name. A `Record` over the type, so a class added in Rust (and
 * regenerated) fails to compile here until it is listed, and the walk below then covers it.
 */
const KINDS: Record<IpcErrorKind, true> = {
  unsupportedFormat: true,
  unsupportedChannels: true,
  corrupt: true,
  io: true,
  cancelled: true,
  notDjSafe: true,
  drmProtected: true,
  wouldClip: true,
  modelUnavailable: true,
  invalidArgument: true,
  internal: true,
  rekordboxUsbExport: true,
  inPlaceRefused: true,
  noSpace: true,
  verifyFailed: true,
  fileChanged: true,
  nothingToUndo: true,
  alreadyExists: true,
  listedTwice: true,
  sameOutputName: true,
  recoveryRunning: true,
  busy: true,
  settingsChanged: true,
};
const ALL = Object.keys(KINDS) as IpcErrorKind[];

/**
 * Where each class can reach the user: the row of a file an export or a revert failed or refused,
 * the confirm step (a file the plan refuses or cannot plan, or the plan itself), and the banner
 * (a job that could not start, the XML, a setting refused, the start-up recovery and Forget).
 */
const USED_ON: Record<IpcErrorKind, readonly ('row' | 'confirm' | 'banner')[]> = {
  unsupportedFormat: ['row', 'confirm'],
  unsupportedChannels: ['row', 'confirm'],
  corrupt: ['row', 'confirm', 'banner'],
  io: ['row', 'confirm', 'banner'],
  cancelled: ['row', 'banner'],
  notDjSafe: ['row', 'confirm'],
  drmProtected: ['row', 'confirm'],
  wouldClip: ['row', 'confirm'],
  modelUnavailable: ['row', 'banner'],
  invalidArgument: ['row', 'confirm', 'banner'],
  internal: ['row', 'confirm', 'banner'],
  rekordboxUsbExport: ['row', 'confirm', 'banner'],
  inPlaceRefused: ['row', 'confirm'],
  noSpace: ['row', 'confirm', 'banner'],
  verifyFailed: ['row', 'banner'],
  fileChanged: ['row', 'confirm', 'banner'],
  nothingToUndo: ['row'],
  alreadyExists: ['row', 'confirm', 'banner'],
  listedTwice: ['row', 'confirm'],
  sameOutputName: ['row', 'confirm'],
  recoveryRunning: ['row', 'confirm', 'banner'],
  busy: ['row', 'confirm', 'banner'],
  settingsChanged: ['confirm', 'banner'],
};

describe('refusal copy', () => {
  it.each(ALL)('%s has a what-to-do line on every surface it reaches', (kind) => {
    const has = surfacesOf(kind);
    for (const surface of USED_ON[kind]) {
      expect(has, `${kind} on ${surface}`).toContain(surface);
      const line = todoFor(kind)[surface] ?? '';
      expect(line.trim().length, `${kind} on ${surface}`).toBeGreaterThan(0);
      // A sentence of its own, in sentence case, never shouting or apologising.
      expect(line).toMatch(/^[A-Z].*\.$/);
      expect(line).not.toMatch(/sorry|oops|please/i);
    }
  });

  it.each(ALL)("%s keeps the engine's why and adds its what-to-do, on every surface", (kind) => {
    for (const surface of SURFACES) {
      const r = refusal({ kind, message: 'something about the file', fileId: 1 }, surface);
      expect(r.why).toBe('Something about the file');
      expect(r.todo.length).toBeGreaterThan(0);
    }
  });

  it('says it of this file on its own row', () => {
    const r = refusal(
      { kind: 'verifyFailed', message: '/m/a.aiff did not verify', fileId: 1 },
      'row',
      '/m/a.aiff',
    );
    expect(r.why).toBe('This file did not verify');
  });

  it("takes the engine's own advice where it words one, which is more exact", () => {
    const locked = refusal(
      {
        kind: 'inPlaceRefused',
        message:
          '/m/a.wav cannot be changed in place: it is locked (Finder: Get Info > Locked). Unlock it, or write a copy to a folder',
        fileId: 1,
      },
      'row',
      '/m/a.wav',
    );
    expect(locked).toEqual({
      why: 'This file cannot be changed in place: it is locked (Finder: Get Info > Locked)',
      todo: 'Unlock it, or write a copy to a folder.',
    });
    // Without it, the line here.
    const bare = refusal(
      { kind: 'inPlaceRefused', message: '/m/a.wav cannot be changed in place', fileId: 1 },
      'confirm',
    );
    expect(bare.todo).toBe(todoFor('inPlaceRefused').confirm);
  });

  it('moves the advice out of the why, so it is said once', () => {
    const space = refusal(
      {
        kind: 'noSpace',
        message:
          'not enough free space on /Volumes/Stick: 90 bytes needed, 10 free. Free some space or choose another folder',
        fileId: null,
      },
      'banner',
    );
    expect(space.why).toBe('Not enough free space on /Volumes/Stick: 90 bytes needed, 10 free');
    expect(space.todo).toBe('Free some space or choose another folder.');
  });

  it.each([
    '/Music/Artist feat. Rihanna.wav',
    '/Music/Mr. Brightside.wav',
    '/Music/Dr. Dre - Still D.R.E. Remix.aiff',
    '/Music/Mr. Brightside/Live. Then Encore.wav',
  ])('never splits a path that holds ". " and a capital: %s', (path) => {
    const locked = refusal(
      {
        kind: 'inPlaceRefused',
        message: `${path} cannot be changed in place: it is locked (Finder: Get Info > Locked). Unlock it, or write a copy to a folder`,
        fileId: 1,
      },
      'confirm',
    );
    expect(locked).toEqual({
      why: `${path.charAt(0).toUpperCase()}${path.slice(1)} cannot be changed in place: it is locked (Finder: Get Info > Locked)`,
      todo: 'Unlock it, or write a copy to a folder.',
    });
    expect(refusal({ kind: 'inPlaceRefused', message: `${path} cannot be changed in place: it is locked`, fileId: 1 }, 'row', path)).toEqual({
      why: 'This file cannot be changed in place: it is locked',
      todo: 'Fix that and export it again, or export it To folder.',
    });
    const usb = refusal(
      {
        kind: 'rekordboxUsbExport',
        message: `${path} is on a rekordbox USB export (/Volumes/Mr. DJ); SoundCheck does not change files there. Process the files in your library, then export again from rekordbox`,
        fileId: 1,
      },
      'row',
      path,
    );
    expect(usb).toEqual({
      why: 'This file is on a rekordbox USB export (/Volumes/Mr. DJ); SoundCheck does not change files there',
      todo: 'Process the files in your library, then export again from rekordbox.',
    });
  });

  it('finds the space advice after the numbers, whatever the volume is called', () => {
    const space = refusal(
      {
        kind: 'noSpace',
        message: 'not enough free space on /Volumes/Mr. Brightside: 90 bytes needed, 10 free. Free some space or choose another folder',
        fileId: null,
      },
      'banner',
    );
    expect(space).toEqual({
      why: 'Not enough free space on /Volumes/Mr. Brightside: 90 bytes needed, 10 free',
      todo: 'Free some space or choose another folder.',
    });
  });

  it('never leaves the why empty', () => {
    expect(refusal({ kind: 'internal', message: '', fileId: null }, 'banner').why).toBe(
      'The command failed',
    );
  });
});
