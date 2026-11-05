import type { ExportPreviewRow, ExportRow } from '@/lib/ipc';
import { exportRow, previewRow } from '@/test/fixtures';
import { countPreview, shortPath, summaryLine } from './summary';

function write(gainDb: number, cut: ExportRow['action'] extends infer A ? (A extends { cut: infer C } ? C : never) : never) {
  return exportRow({ action: { type: 'write', gainDb, cut } });
}

function rows(list: (ExportRow | null)[]): ExportPreviewRow[] {
  return list.map((row, i) => previewRow(i + 1, row));
}

const cut = { type: 'cut', frames: 100, seconds: 0.21 } as const;
const library = { type: 'library' } as const;
const skip = exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'silent' } } });
const xml = exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } } });

const backups = { backupFolder: '/Users/dj/Music/SoundCheck Backups' };

describe('the confirm step numbers', () => {
  it('says what will happen, as the plan says it', () => {
    const p = rows([
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
      written: 5,
    });
    expect(summaryLine(p, backups, null)).toBe(
      '9 files: 2 gain, 2 cut, 1 grid only, 1 tags only, 1 XML only, 2 skipped, 1 unreadable; backups in ~/Music/SoundCheck Backups',
    );
  });

  it('counts a file with both a gain and a cut under both', () => {
    const c = countPreview(rows([write(-3, cut)]));
    expect(c).toMatchObject({ files: 1, gain: 1, cut: 1, written: 1 });
  });

  it('keeps the longest cut, to say how far positions move', () => {
    const long = { type: 'cut', frames: 400, seconds: 0.48 } as const;
    expect(countPreview(rows([write(0, cut), write(0, long)])).longestCutS).toBe(0.48);
  });

  it('names where copies go, in one line, singular for one', () => {
    expect(summaryLine(rows([write(-1, library)]), { backupFolder: null }, '/Users/dj/Out')).toBe(
      '1 file: 1 gain; copies in ~/Out',
    );
    expect(summaryLine([], { backupFolder: null }, null)).toBe('0 files: nothing to do');
  });

  it('writes nothing when every file is skipped', () => {
    expect(countPreview(rows([skip])).written).toBe(0);
  });

  it('counts an XML-only row as new work only when it is not going along again', () => {
    const list = rows([xml, xml]);
    expect(countPreview(list).xmlOnlyNew).toBe(2);
    expect(countPreview(list, [1]).xmlOnlyNew).toBe(1);
    expect(countPreview(list, [1, 2]).xmlOnly).toBe(2);
  });

  it('counts copies replaced and files that are not ours', () => {
    const foreign = exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'otherFileInFolder' } } });
    const list = [previewRow(1, write(-1, library), { replacesCopy: true }), previewRow(2, foreign)];
    expect(countPreview(list)).toMatchObject({ replacing: 1, foreign: 1, skipped: 1, written: 1 });
  });

  it('shortens a path under the home folder only', () => {
    expect(shortPath('/Users/dj/Music')).toBe('~/Music');
    expect(shortPath('/Volumes/USB/Music')).toBe('/Volumes/USB/Music');
  });
});

describe('rows an export in place refuses', () => {
  const refused = (kind: 'inPlaceRefused' | 'rekordboxUsbExport' | 'io', fileId: number): ExportPreviewRow => ({
    fileId,
    row: null,
    error: { kind, message: 'no', fileId },
    identity: null,
    replacesCopy: false,
  });

  it('are counted as refused, not as unreadable', () => {
    const rows = [refused('inPlaceRefused', 1), refused('rekordboxUsbExport', 2), refused('io', 3)];
    const counts = countPreview(rows);
    expect(counts.refused).toBe(2);
    expect(counts.unreadable).toBe(1);
    expect(counts.written).toBe(0);
    expect(summaryLine(rows, { backupFolder: '/b' }, null)).toBe('3 files: 1 unreadable, 2 refused; backups in /b');
  });
});
