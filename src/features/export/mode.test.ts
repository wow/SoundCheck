import type { Row } from '@/state/library';
import { entry } from '@/test/fixtures';
import { batchChoice, folderOf } from './mode';

function row(path: string, serato: boolean): Row {
  const e = entry(1);
  return { entry: { ...e, path, info: { ...e.info, serato } }, state: 'analysed', progress: 1 };
}

describe('the batch mode', () => {
  it('is the chosen one when no file holds Serato data', () => {
    expect(batchChoice({ batchMode: 'prepare', seratoPrepareFolders: [] }, [row('/a/x.wav', false)])).toMatchObject({
      mode: 'prepare',
      auto: false,
      kept: false,
    });
    expect(batchChoice({ batchMode: 'library', seratoPrepareFolders: [] }, []).mode).toBe('library');
  });

  it('switches to Library for a Serato file in a folder with no choice, leaving the saved mode', () => {
    const s = { batchMode: 'prepare' as const, seratoPrepareFolders: [] };
    const choice = batchChoice(s, [row('/a/x.wav', true), row('/a/y.wav', true), row('/b/z.wav', false)]);
    expect(choice).toMatchObject({ mode: 'library', auto: true, seratoTracks: 2, seratoFolders: ['/a'] });
    expect(s.batchMode).toBe('prepare');
    // With those tracks gone, the next batch is Prepare again.
    expect(batchChoice(s, [row('/c/new.wav', false)]).mode).toBe('prepare');
  });

  it('keeps Prepare for a folder the user chose it for', () => {
    const s = { batchMode: 'prepare' as const, seratoPrepareFolders: ['/a'] };
    expect(batchChoice(s, [row('/a/x.wav', true)])).toMatchObject({ mode: 'prepare', auto: false, kept: true });
    // Another folder has no choice yet.
    expect(batchChoice(s, [row('/a/x.wav', true), row('/b/y.wav', true)]).mode).toBe('library');
  });

  it('says nothing when Library was chosen anyway', () => {
    expect(batchChoice({ batchMode: 'library', seratoPrepareFolders: [] }, [row('/a/x.wav', true)])).toMatchObject({
      mode: 'library',
      auto: false,
      kept: false,
    });
  });

  it('finds the folder of a path', () => {
    expect(folderOf('/Music/Set/a.wav')).toBe('/Music/Set');
    expect(folderOf('C:\\Music\\a.wav')).toBe('C:\\Music');
  });
});
