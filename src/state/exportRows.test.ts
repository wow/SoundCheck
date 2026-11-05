import type { IpcError, RevertResult, RowUpdate } from '@/lib/ipc';
import { analysed, analysis, artefacts, doneRow, entry, exportRow, plan, verification } from '@/test/fixtures';
import { counts, exportableIds, isLocked, staleIds, useLibrary, visibleIds, xmlOnlyIds } from './library';

const initial = useLibrary.getState();
const lib = () => useLibrary.getState();
const row = (id: number) => {
  const r = lib().rows[id];
  if (!r) throw new Error(`no row ${id}`);
  return r;
};

const update = (fileId: number): RowUpdate => ({
  fileId,
  row: analysis({ shortTermP95: -11 }),
  plan: plan({ measured: -11, gain: { type: 'atTarget' } }),
  revision: 0,
});

/** Row 1 (and any others) analysed, and an export of them running. */
function exporting(...ids: number[]) {
  lib().add(ids.map((id) => entry(id)));
  for (const id of ids) lib().applyEvent(analysed(id));
  lib().beginExport(ids, []);
}

beforeEach(() => useLibrary.setState(initial, true));

describe('rows during an export', () => {
  it('walks Processing, Writing, Checking and Done, verified', () => {
    exporting(1);
    expect(row(1).state).toBe('exportQueued');
    lib().applyEvent({ type: 'started', jobId: 3, fileId: 1 });
    expect(row(1).state).toBe('processing');
    expect(lib().job).toMatchObject({ kind: 'export', jobId: 3 });
    lib().applyEvent({ type: 'progress', jobId: 3, fileId: 1, fraction: 0.5 });
    expect(row(1)).toMatchObject({ state: 'processing', progress: 0.5 });
    lib().applyEvent({ type: 'processing', jobId: 3, fileId: 1, row: exportRow() });
    expect(row(1).state).toBe('writing');
    const written = exportRow({ stage: 'writing', verification: verification(), revertible: true });
    lib().applyEvent({ type: 'written', jobId: 3, fileId: 1, row: written });
    expect(row(1)).toMatchObject({ state: 'checking', revertible: true });
    lib().applyEvent({
      type: 'exportDone',
      jobId: 3,
      fileId: 1,
      row: doneRow(),
      inPlace: true,
      update: update(1),
    });
    expect(row(1).state).toBe('done');
    expect(row(1).export?.gridCheck?.result).toBe('pass');
  });

  it('shows what the confirm step planned while a row waits', () => {
    lib().add([entry(1)]);
    lib().applyEvent(analysed(1));
    const planned = exportRow({ action: { type: 'write', gainDb: -1, cut: { type: 'library' } } });
    lib().beginExport([1], [{ fileId: 1, row: planned, error: null }]);
    expect(row(1).export).toEqual(planned);
  });

  it('locks exactly the files of the batch until it finishes', () => {
    exporting(1, 2);
    lib().add([entry(3)]);
    expect(isLocked(lib().locked, 1)).toBe(true);
    expect(isLocked(lib().locked, 2)).toBe(true);
    expect(isLocked(lib().locked, 3)).toBe(false);
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    expect(isLocked(lib().locked, 1)).toBe(false);
  });

  it('a skipped file says so, with the words in its row', () => {
    exporting(1);
    const skipped = exportRow({
      stage: 'skipped',
      action: { type: 'skip', reason: { type: 'silent' } },
      positionsShift: false,
      notes: ['it has no audio above the -70 LUFS gate'],
    });
    lib().applyEvent({ type: 'exportSkipped', jobId: 3, fileId: 1, row: skipped });
    expect(row(1).state).toBe('skipped');
    expect(row(1).export?.notes).toEqual(['it has no audio above the -70 LUFS gate']);
    expect(row(1).analysis).toBeDefined();
  });

  it('a failure keeps the row\'s numbers and the export standing before it', () => {
    const standing = doneRow();
    lib().add([entry(1)]);
    lib().applyEvent(analysed(1));
    lib().beginExport([1], []);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: standing, inPlace: false, update: null });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });

    lib().beginExport([1], [{ fileId: 1, row: exportRow(), error: null }]);
    const error: IpcError = { kind: 'verifyFailed', message: 'no match', fileId: 1 };
    lib().applyEvent({ type: 'processing', jobId: 4, fileId: 1, row: exportRow() });
    lib().applyEvent({ type: 'failed', jobId: 4, fileId: 1, error });
    expect(row(1)).toMatchObject({ state: 'failed', exportError: error, export: standing });
    expect(row(1).analysis).toBeDefined();
    // A failed export's row is not an analysis to redo.
    expect(staleIds(lib(), [70, 180])).toEqual([]);
  });

  it('cancel puts a row back as it was, and rows never reached too', () => {
    exporting(1, 2);
    lib().applyEvent({ type: 'cancelled', jobId: 3, fileId: 1 });
    expect(row(1).state).toBe('analysed');
    expect(row(1).export).toBeUndefined();
    expect(row(2).state).toBe('exportQueued');
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: true });
    expect(row(2).state).toBe('analysed');
    expect(lib().lastExport).toMatchObject({ written: 0, cancelled: true });
    expect(staleIds(lib(), [70, 180])).toEqual([]);
  });

  it('counts what became of the batch and the Done chip', () => {
    exporting(1, 2, 3);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: update(1) });
    lib().applyEvent({
      type: 'exportSkipped',
      jobId: 3,
      fileId: 2,
      row: exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } } }),
    });
    lib().applyEvent({ type: 'failed', jobId: 3, fileId: 3, error: { kind: 'io', message: 'x', fileId: 3 } });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    expect(lib().lastExport).toMatchObject({ written: 1, xmlOnly: 1, skipped: 0, failed: 1, cancelled: false });
    expect(lib().lastRun).toBe('export');
    expect(counts(lib()).done).toBe(1);
    lib().setFilter('done');
    expect(visibleIds(lib())).toEqual([1]);
  });

  it('keeps Done through a replan, and shows Needs review when the grid still asks for it', () => {
    exporting(1);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: update(1) });
    lib().applyReplan({ revision: 5, plans: [{ fileId: 1, plan: plan() }] });
    expect(row(1).state).toBe('done');
    lib().applyReplan({
      revision: 6,
      plans: [{ fileId: 1, plan: plan({ status: 'needsReview', review: [{ type: 'drifts' }] }) }],
    });
    expect(row(1).state).toBe('needsReview');
  });

  it('a replan leaves a row in flight in its stage', () => {
    exporting(1);
    lib().applyEvent({ type: 'processing', jobId: 3, fileId: 1, row: exportRow() });
    lib().applyReplan({ revision: 5, plans: [{ fileId: 1, plan: plan() }] });
    expect(row(1).state).toBe('writing');
  });

  it('a job refused before it started leaves its rows as they were and says why', () => {
    exporting(1);
    const error: IpcError = { kind: 'busy', message: 'being analysed', fileId: 1 };
    lib().applyEvent({ type: 'aborted', jobId: 0, error });
    lib().applyEvent({ type: 'finished', jobId: 0, cancelled: false });
    expect(lib().aborted).toEqual(error);
    expect(lib().abortedKind).toBe('export');
    expect(row(1).state).toBe('analysed');
    expect(lib().locked).toBeNull();
  });
});

describe('exportDone by case', () => {
  it('in place with an update: the row is the written file', () => {
    exporting(1);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: update(1) });
    expect(row(1).analysis?.shortTermP95).toBe(-11);
    expect(row(1).plan?.gain).toEqual({ type: 'atTarget' });
  });

  it('in place without an update: the numbers are gone and the row waits for an analysis', () => {
    exporting(1);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: null });
    expect(row(1)).toMatchObject({ state: 'queued', analysis: undefined, plan: undefined });
    expect(row(1).export?.stage).toBe('done');
    expect(row(1).revertible).toBe(true);
    // Its next analysis, after the export, lands on a row that keeps its export: Done again.
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    expect(row(1).state).toBe('queued');
    lib().queue([1], [70, 180]);
    lib().applyEvent(analysed(1));
    expect(row(1).state).toBe('done');
    expect(row(1).export?.stage).toBe('done');
  });

  it('a copy in a folder without an update keeps the row', () => {
    exporting(1);
    const before = row(1).analysis;
    lib().applyEvent({
      type: 'exportDone',
      jobId: 3,
      fileId: 1,
      row: doneRow({ revertible: false }),
      inPlace: false,
      update: null,
    });
    expect(row(1).analysis).toBe(before);
    expect(row(1)).toMatchObject({ state: 'done', revertible: false });
  });

  it('in place, the row now tells the written depth', () => {
    exporting(1);
    lib().applyEvent({
      type: 'exportDone',
      jobId: 3,
      fileId: 1,
      row: doneRow({ verification: verification({ bitsOut: 24 }) }),
      inPlace: true,
      update: update(1),
    });
    expect(row(1).entry.info.bitsPerSample).toBe(24);
  });
});

describe('artefacts', () => {
  it('keeps the links, and a failure instead of them', () => {
    exporting(1);
    const a = artefacts();
    lib().applyEvent({ type: 'artefacts', jobId: 3, artefacts: a });
    expect(lib().artefacts).toEqual(a);
    const error: IpcError = { kind: 'io', message: 'disk full', fileId: null };
    lib().applyEvent({ type: 'artefactsFailed', jobId: 3, error });
    expect(lib()).toMatchObject({ artefacts: null, artefactsFailed: error });
  });

  it('a new export drops the last one\'s links', () => {
    exporting(1);
    lib().applyEvent({ type: 'artefacts', jobId: 3, artefacts: artefacts() });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    lib().beginExport([1], []);
    expect(lib().artefacts).toBeNull();
  });

  it('notes why the XML button left rows out', () => {
    lib().add([entry(1), entry(2)]);
    lib().applyXml({
      artefacts: artefacts({ listed: 1 }),
      notListed: [{ fileId: 2, why: 'no grid' }],
    });
    expect(lib().xmlLeftOut).toBe(1);
    expect(row(2).xmlNotListed).toBe('no grid');
    expect(row(1).xmlNotListed).toBeUndefined();
  });
});

describe('which rows the buttons act on', () => {
  it('Export takes analysed rows with no finished export; a saved grid brings one back', () => {
    lib().add([entry(1), entry(2), entry(3)]);
    lib().applyEvent(analysed(1));
    lib().applyEvent(analysed(2));
    expect(exportableIds(lib())).toEqual([1, 2]);
    expect(xmlOnlyIds(lib())).toEqual([1, 2]);
    lib().beginExport([1], []);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: update(1) });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    expect(exportableIds(lib())).toEqual([2]);
    expect(xmlOnlyIds(lib())).toEqual([2]);
    lib().applyRowUpdate(update(1));
    expect(row(1).exportStale).toBe(true);
    expect(exportableIds(lib())).toEqual([1, 2]);
    expect(row(1).state).toBe('done');
  });

  it('a skipped or failed row can be exported again', () => {
    exporting(1, 2);
    lib().applyEvent({
      type: 'exportSkipped',
      jobId: 3,
      fileId: 1,
      row: exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'silent' } } }),
    });
    lib().applyEvent({ type: 'failed', jobId: 3, fileId: 2, error: { kind: 'io', message: 'x', fileId: 2 } });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    expect(exportableIds(lib())).toEqual([1, 2]);
  });
});

describe('revert', () => {
  const result = (overrides: Partial<RevertResult> = {}): RevertResult => ({
    entry: entry(1, 'Reverted'),
    update: update(1),
    backup: '/backups/a',
    earlierChanges: 0,
    notes: [],
    ...overrides,
  });

  function done() {
    exporting(1);
    lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: true, update: update(1) });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
  }

  it('brings the original back: no export, nothing left to revert', () => {
    done();
    lib().setReverting(1);
    expect(row(1).reverting).toBe(true);
    lib().applyRevert(1, result());
    expect(row(1)).toMatchObject({ reverting: false, revertible: false, state: 'analysed' });
    expect(row(1).export).toBeUndefined();
    expect(row(1).entry.info.title).toBe('Reverted');
  });

  it('an earlier change still standing stays revertible', () => {
    done();
    lib().applyRevert(1, result({ earlierChanges: 2 }));
    expect(row(1).revertible).toBe(true);
  });

  it('with no update: the row is cleared and waits for an analysis, with why', () => {
    done();
    lib().applyRevert(1, result({ update: null, notes: ['could not be analysed'] }));
    expect(row(1)).toMatchObject({
      state: 'queued',
      analysis: undefined,
      plan: undefined,
      revertNotes: ['could not be analysed'],
    });
    expect(staleIds(lib(), [70, 180])).toEqual([1]);
  });

  it('a refusal stops the spinner and leaves the export standing', () => {
    done();
    lib().setReverting(1);
    const error: IpcError = { kind: 'fileChanged', message: 'edited since', fileId: 1 };
    lib().revertFailed(1, error);
    expect(row(1)).toMatchObject({ reverting: false, revertError: error, state: 'done' });
    expect(row(1).export?.stage).toBe('done');
  });

  it('a row reverting is not picked up by Analyse', () => {
    lib().add([entry(1)]);
    lib().setReverting(1);
    expect(staleIds(lib(), [70, 180])).toEqual([]);
  });
});

describe('a reload during an export', () => {
  const session = (overrides = {}) => ({
    revision: 4,
    rows: [
      { entry: entry(1), row: analysis(), plan: plan(), export: exportRow({ stage: 'writing', verification: verification() }), revertible: true },
      { entry: entry(2), row: analysis(), plan: plan(), export: exportRow(), revertible: false },
      { entry: entry(3), row: analysis(), plan: plan(), export: doneRow(), revertible: true },
      { entry: entry(4), row: analysis(), plan: plan(), export: null, revertible: false },
    ],
    runningJobs: [9],
    artefacts: null,
    artefactsFailed: null,
    ...overrides,
  });

  it('shows the rows in flight in their stage, and every row locked', () => {
    lib().restore(session(), [70, 180]);
    expect(row(1).state).toBe('checking');
    expect(row(2).state).toBe('writing');
    expect(row(3).state).toBe('done');
    expect(row(4).state).toBe('analysed');
    expect(isLocked(lib().locked, 4)).toBe(true);
  });

  it('keeps Revert after a reload', () => {
    lib().restore(session({ runningJobs: [] }), [70, 180]);
    expect(row(3).revertible).toBe(true);
    expect(row(3).export?.verification).toBeDefined();
    expect(isLocked(lib().locked, 3)).toBe(false);
  });

  it('a read with no job left ends the rows in flight and brings the artefact links back', () => {
    lib().restore(session(), [70, 180]);
    lib().select(3);
    const a = artefacts();
    lib().restore(
      session({
        runningJobs: [],
        artefacts: a,
        rows: [
          { entry: entry(1), row: analysis(), plan: plan(), export: doneRow(), revertible: true },
          { entry: entry(2), row: analysis(), plan: plan(), export: null, revertible: false },
          { entry: entry(3), row: analysis(), plan: plan(), export: doneRow(), revertible: true },
          { entry: entry(4), row: analysis(), plan: plan(), export: null, revertible: false },
        ],
      }),
      [70, 180],
    );
    lib().windDownEnded();
    expect(row(1).state).toBe('done');
    expect(row(2).state).toBe('analysed');
    expect(lib().selected).toBe(3);
    expect(lib().artefacts).toEqual(a);
    expect(lib().locked).toBeNull();
    expect(lib().lastExport).toMatchObject({ written: 2, stoppedByReload: true, seconds: null });
  });

  it('a row left in flight with no job running shows no stage it is not in', () => {
    lib().restore(session({ runningJobs: [] }), [70, 180]);
    expect(row(1).state).toBe('analysed');
    expect(row(1).export).toBeUndefined();
  });

  it('a done row the session holds no analysis for is analysed again, keeping its export', () => {
    lib().restore(
      session({
        runningJobs: [],
        rows: [{ entry: entry(1), row: null, plan: null, export: doneRow(), revertible: true }],
      }),
      [70, 180],
    );
    expect(row(1)).toMatchObject({ state: 'cancelled', revertible: true });
    expect(row(1).export?.stage).toBe('done');
    expect(staleIds(lib(), [70, 180])).toEqual([1]);
  });

  it('brings back the artefacts failure too', () => {
    const error: IpcError = { kind: 'io', message: 'disk full', fileId: null };
    lib().restore(session({ runningJobs: [], artefactsFailed: error }), [70, 180]);
    expect(lib().artefactsFailed).toEqual(error);
  });

  it('a track added while the job winds down stays queued through the next read', () => {
    lib().restore(session(), [70, 180]);
    lib().add([entry(5)]);
    const rows = [...session().rows, { entry: entry(5), row: null, plan: null, export: null, revertible: false }];
    lib().restore(session({ rows }), [70, 180]);
    expect(row(5).state).toBe('queued');
  });
});
