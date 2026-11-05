import type { ExportedWith, ExportRow } from '@/lib/ipc';
import type { Row } from '@/state/library';
import { analysis, doneRow, entry, exportRow, plan } from '@/test/fixtures';
import { againCopy, exportDifference, exportWork, type ExportNow } from './work';

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

  it('a skipped row goes along, but makes no work; under other settings it may not be skipped', () => {
    expect(work([skip(same)])).toMatchObject({ fresh: [], along: [1] });
    expect(work([skip({ ...same, settings: { ...now.settings, place: 'inPlace' }, outDir: null })]).fresh).toEqual([1]);
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
