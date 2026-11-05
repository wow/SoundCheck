import type { ExportedWith, ExportRow, ExportSkip } from '@/lib/ipc';
import type { Row } from '@/state/library';
import { analysis, doneRow, entry, exportRow, plan } from '@/test/fixtures';
import { againCopy, exportDifference, exportWork, skipCure, type ExportNow } from './work';

const now: ExportNow = {
  decide: { mode: 'dj', target: -11, ceiling: -0.5, bpmRange: [70, 180] },
  settings: { batchMode: 'prepare', place: 'folder', depth: null, gridOnly: false, tbpm: true, xml: true, leadMs: 5 },
  outDir: '/Users/dj/Out',
};
const same: ExportedWith = now;

function row(exp?: ExportRow, overrides: Partial<Row> = {}): Row {
  return {
    entry: entry(1),
    state: exp?.stage === 'done' ? 'done' : 'analysed',
    progress: 1,
    analysis: analysis(),
    plan: plan(),
    analysedRange: [70, 180],
    ...(exp ? { export: exp } : {}),
    ...overrides,
  };
}

const done = (w: ExportedWith) => row(doneRow({ exportedWith: w, revertible: false }));
const different = (w: Partial<ExportedWith>) => exportDifference(done({ ...same, ...w }), now)?.text;

const skip = (w: ExportedWith) =>
  row(exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'silent' } }, exportedWith: w }));
const xml = (w: ExportedWith) =>
  row(exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } }, exportedWith: w }));

const work = (rows: Row[]) => exportWork(rows, rows.map((_, i) => i + 1), now);

describe('how a row was exported, against the settings now', () => {
  it('says nothing when they match', () => {
    expect(exportDifference(done(same), now)).toBeNull();
  });

  it('tells the target, the folder, the depth and the mode', () => {
    expect(different({ decide: { ...now.decide, target: -8 } })).toBe('Exported at -8.0');
    expect(different({ outDir: '/Users/dj/Old' })).toBe('Exported to ~/Old');
    expect(different({ settings: { ...now.settings, place: 'inPlace' }, outDir: null })).toBe('Exported in place');
    expect(different({ settings: { ...now.settings, depth: 24 } })).toBe('Exported at 24-bit');
    expect(different({ settings: { ...now.settings, batchMode: 'library' } })).toBe('Exported as Library');
    expect(different({ decide: { ...now.decide, ceiling: -1 } })).toBe('Exported with ceiling -1.0');
  });

  it('a grid-only export has no gain to differ', () => {
    const gridOnly = { ...now.settings, gridOnly: true };
    const a = { ...now, settings: gridOnly };
    expect(exportDifference(done({ ...a, decide: { ...now.decide, target: -8 } }), a)).toBeNull();
  });

  it('a file only the XML carries differs only by where it is listed', () => {
    expect(exportDifference(xml({ ...same, decide: { ...now.decide, target: -8 } }), now)).toBeNull();
    expect(exportDifference(xml({ ...same, settings: { ...now.settings, depth: 24 } }), now)).toBeNull();
    expect(exportDifference(xml({ ...same, outDir: '/Old' }), now)?.kind).toBe('place');
  });

  it('a row still running has no history to compare', () => {
    expect(exportDifference(row(exportRow({ exportedWith: { ...same, outDir: '/Old' } })), now)).toBeNull();
  });
});

describe('what Export has to work on', () => {
  it('new rows are work; exported ones matching the settings are not', () => {
    expect(work([row(), done(same)])).toMatchObject({ fresh: [1], differing: [], anyExported: true });
  });

  it('rows exported at another target stay, apart from the new ones', () => {
    const w = work([row(), done({ ...same, decide: { ...now.decide, target: -8 } })]);
    expect(w).toMatchObject({ fresh: [1], differing: [2] });
  });

  describe('a skipped row', () => {
    const skipped = (reason: ExportSkip, w: ExportedWith = same) =>
      row(exportRow({ stage: 'skipped', action: { type: 'skip', reason }, exportedWith: w }));
    const moved = { ...same, settings: { ...now.settings, place: 'inPlace' as const }, outDir: null };
    const target = { ...same, decide: { ...now.decide, target: -8 } };
    const codec = 'mp3' as const;

    it('about the file itself stays along, whatever changed', () => {
      const reasons: ExportSkip[] = [
        { type: 'notDjSafeRate', sampleRateHz: 96_000 },
        { type: 'unsupportedChannels', channels: 6 },
        { type: 'unsupported', codec: 'alac' },
        { type: 'silent' },
      ];
      for (const reason of reasons) {
        expect(skipCure(reason)).toBe('file');
        for (const w of [same, target, moved]) expect(work([skipped(reason, w)]).along).toEqual([1]);
        expect(work([{ ...skipped(reason), exportStale: true }]).along).toEqual([1]);
      }
    });

    it('for its grid is planned again once a grid was saved or confirmed, or settings changed', () => {
      const reasons: ExportSkip[] = [
        { type: 'noGrid' },
        { type: 'gridNeedsReview' },
        { type: 'xmlNoGrid', codec },
        { type: 'xmlGridNeedsReview', codec },
        { type: 'xmlNotOptedIn', codec },
      ];
      for (const reason of reasons) {
        expect(skipCure(reason)).toBe('grid');
        expect(work([skipped(reason)]).along).toEqual([1]);
        expect(work([{ ...skipped(reason), exportStale: true }]).retry).toEqual([1]);
        expect(work([skipped(reason, moved)]).retry).toEqual([1]);
      }
    });

    it('for its settings is planned again only when they changed', () => {
      const reasons: ExportSkip[] = [
        { type: 'seratoInPlaceCut', cutS: 0.1 },
        { type: 'seratoUnknownInPlaceCut', cutS: 0.1 },
        { type: 'nothingToWrite', reason: { type: 'gridOnlyFlac' } },
      ];
      for (const reason of reasons) {
        expect(skipCure(reason)).toBe('settings');
        expect(work([skipped(reason)]).along).toEqual([1]);
        expect(work([skipped(reason, moved)]).retry).toEqual([1]);
      }
    });

    it('for what is in the folder is always planned again, since that can change unseen', () => {
      for (const reason of [{ type: 'otherFileInFolder' }, { type: 'copyOfOtherTrack' }] as ExportSkip[]) {
        expect(skipCure(reason)).toBe('disk');
        expect(work([skipped(reason)]).retry).toEqual([1]);
      }
    });

    it('is never counted as new work', () => {
      expect(work([skipped({ type: 'otherFileInFolder' })]).fresh).toEqual([]);
    });
  });

  it('an XML-only row makes no work unless its grid changed', () => {
    expect(work([xml(same)])).toMatchObject({ fresh: [], along: [1] });
    expect(work([{ ...xml(same), exportStale: true }]).fresh).toEqual([1]);
  });

  it('a saved grid brings an exported row back', () => {
    expect(work([{ ...done(same), exportStale: true }]).fresh).toEqual([1]);
  });

  it('a row not analysed is no part of it', () => {
    expect(work([row(undefined, { analysis: undefined })])).toMatchObject({ fresh: [], along: [], differing: [] });
  });

  it('counts rows analysed under another BPM range, not those that stay', () => {
    const old: [number, number] = [60, 120];
    expect(work([row(undefined, { analysedRange: old })]).needAnalyse).toBe(1);
    expect(work([{ ...done(same), analysedRange: old }]).needAnalyse).toBe(0);
    // Neither does a row that is only going along.
    expect(work([{ ...skip(same), analysedRange: old }]).needAnalyse).toBe(0);
    expect(work([row(undefined, { analysedRange: undefined })]).needAnalyse).toBe(0);
  });

  it('knows whether a revert exists', () => {
    expect(work([done(same)]).anyRevertible).toBe(false);
    expect(work([{ ...done(same), revertible: true }]).anyRevertible).toBe(true);
  });
});

describe('what the confirm step says about rows that stay', () => {
  it('names the one way they differ', () => {
    const rows = [1, 2].map(() => done({ ...same, decide: { ...now.decide, target: -8 } }));
    expect(againCopy(rows, now)).toEqual({
      stays: '2 exported at -8.0 stay as they are',
      offer: 'Export them again at -11.0',
    });
  });

  it('offers the folder when that is what differs', () => {
    const rows = [done({ ...same, outDir: '/Users/dj/Old' })];
    expect(againCopy(rows, now).offer).toBe('Export them again to ~/Out');
  });

  it('stays general when they differ in several ways', () => {
    const rows = [done({ ...same, decide: { ...now.decide, target: -8 } }), done({ ...same, outDir: '/Old' })];
    expect(againCopy(rows, now)).toEqual({
      stays: '2 exported with other settings stay as they are',
      offer: 'Export them again with these settings',
    });
  });
});

describe('a row skipped because only the XML could carry its grid', () => {
  const nothing = (w: ExportedWith) =>
    row(
      exportRow({
        stage: 'skipped',
        action: { type: 'skip', reason: { type: 'nothingToWrite', reason: { type: 'gridOnlyFlac' } } },
        exportedWith: w,
      }),
    );
  const xmlOff = { ...same, settings: { ...same.settings, xml: false } };

  it('comes back when the XML is turned on', () => {
    expect(exportDifference(nothing(xmlOff), now)?.text).toBe('Exported without the rekordbox XML');
    const found = work([nothing(xmlOff)]);
    expect(found.retry).toEqual([1]);
    expect(found.along).toEqual([]);
  });

  it('goes along while the XML stays off', () => {
    const off = { ...now, settings: { ...now.settings, xml: false } };
    expect(exportDifference(nothing(xmlOff), off)).toBeNull();
    expect(exportWork([nothing(xmlOff)], [1], off).along).toEqual([1]);
  });

  it('says the difference that matters most when there is another', () => {
    const elsewhere = { ...xmlOff, outDir: '/Users/dj/Old' };
    expect(exportDifference(nothing(elsewhere), now)?.text).toBe('Exported to ~/Old');
  });

  it('the XML does not bring back a row skipped for anything else', () => {
    expect(exportDifference(skip(xmlOff), now)).toBeNull();
    expect(exportDifference(done(xmlOff), now)).toBeNull();
  });
});

describe('a name taken in the batch', () => {
  it('is planned again, since what else goes along can change with no word to the row', () => {
    expect(skipCure({ type: 'sameNameInBatch' })).toBe('disk');
    const taken = row(exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'sameNameInBatch' } }, exportedWith: same }));
    expect(work([taken]).retry).toEqual([1]);
  });
});
