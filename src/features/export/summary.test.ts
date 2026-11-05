import type { ExportPreview, ExportPreviewRow, ExportRow } from '@/lib/ipc';
import { exportRow } from '@/test/fixtures';
import { countPreview, shortPath, summaryLine } from './summary';

function write(gainDb: number, cut: ExportRow['action'] extends infer A ? (A extends { cut: infer C } ? C : never) : never) {
  return exportRow({ action: { type: 'write', gainDb, cut } });
}

function rows(list: (ExportRow | null)[]): ExportPreviewRow[] {
  return list.map((row, i) => ({
    fileId: i + 1,
    row,
    error: row ? null : { kind: 'corrupt', message: 'unreadable', fileId: i + 1 },
  }));
}

const cut = { type: 'cut', frames: 100, seconds: 0.21 } as const;
const library = { type: 'library' } as const;
const skip = exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'silent' } } });
const xml = exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } } });

function preview(list: (ExportRow | null)[], backupFolder: string | null = '/Users/dj/Music/SoundCheck Backups'): ExportPreview {
  return { rows: rows(list), backupFolder, artefactsFolder: null };
}

describe('the confirm step numbers', () => {
  it('says what will happen, as the plan says it', () => {
    const p = preview([
      write(-2, library),
      write(-1, cut),
      write(0, cut),
      write(0, { type: 'gridOnly' }),
      write(0.01, library),
      xml,
      skip,
      skip,
      null,
    ]);
    expect(countPreview(p)).toMatchObject({
      files: 9,
      gain: 2,
      cut: 2,
      gridOnly: 1,
      tagsOnly: 1,
      xmlOnly: 1,
      skipped: 2,
      unreadable: 1,
      toDo: 6,
    });
    expect(summaryLine(p, null)).toBe(
      '9 files: 2 gain, 2 cut, 1 grid only, 1 tags only, 1 XML only, 2 skipped, 1 unreadable; backups in ~/Music/SoundCheck Backups',
    );
  });

  it('counts a file with both a gain and a cut under both', () => {
    const c = countPreview(preview([write(-3, cut)]));
    expect(c).toMatchObject({ files: 1, gain: 1, cut: 1, toDo: 1 });
  });

  it('keeps the longest cut, to say how far positions move', () => {
    const long = { type: 'cut', frames: 400, seconds: 0.48 } as const;
    expect(countPreview(preview([write(0, cut), write(0, long)])).longestCutS).toBe(0.48);
  });

  it('names where copies go, in one line, singular for one', () => {
    expect(summaryLine(preview([write(-1, library)], null), '/Users/dj/Out')).toBe('1 file: 1 gain; copies in ~/Out');
    expect(summaryLine(preview([], null), null)).toBe('0 files: nothing to do');
  });

  it('nothing to do when every file is skipped', () => {
    expect(countPreview(preview([skip])).toDo).toBe(0);
  });

  it('shortens a path under the home folder only', () => {
    expect(shortPath('/Users/dj/Music')).toBe('~/Music');
    expect(shortPath('/Volumes/USB/Music')).toBe('/Volumes/USB/Music');
  });
});
