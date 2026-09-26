import { analysed, analysis, entry, plan } from '@/test/fixtures';
import { counts, staleIds, useLibrary, visibleIds } from './library';

const initial = useLibrary.getState();
beforeEach(() => useLibrary.setState(initial, true));

describe('library store', () => {
  it('walks a row from queued through analysing to its plan status', () => {
    const lib = () => useLibrary.getState();
    lib().add([entry(1), entry(2)]);
    lib().queue([1, 2], [70, 180]);
    expect(lib().rows[1]?.state).toBe('queued');
    lib().applyEvent({ type: 'started', jobId: 7, fileId: 1 });
    lib().applyEvent({ type: 'progress', jobId: 7, fileId: 1, fraction: 0.42 });
    expect(lib().rows[1]).toMatchObject({ state: 'analysing', progress: 0.42 });
    expect(lib().job?.jobId).toBe(7);
    lib().applyEvent(analysed(1, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    expect(lib().rows[1]?.state).toBe('needsReview');
    lib().applyEvent({
      type: 'failed',
      jobId: 7,
      fileId: 2,
      error: { kind: 'corrupt', message: 'truncated', fileId: 2 },
    });
    expect(lib().rows[2]?.state).toBe('error');
    lib().applyEvent({ type: 'finished', jobId: 7, cancelled: false });
    expect(lib().job).toBeNull();
    expect(lib().lastBatch).toMatchObject({ analysed: 1, review: 1, failed: 1, cancelled: false });
  });

  it('counts chips and filters rows', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1, 'Alpha'), entry(2, 'Beta'), entry(3, 'Gamma'), entry(4, 'Delta')]);
    lib.applyEvent(analysed(1));
    lib.applyEvent(analysed(2, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    lib.applyEvent(
      analysed(3, plan({ gain: { type: 'gain', gainDb: 1.2, shortByLu: 1.8, truePeakAfter: -0.5 } })),
    );
    lib.applyEvent(
      analysed(4, plan({ gain: null, skip: { type: 'analyseOnly', codec: 'alac' }, status: 'skipped' })),
    );
    const s = useLibrary.getState();
    expect(counts(s)).toEqual({ all: 4, review: 1, short: 1, skipped: 1, done: 0 });
    useLibrary.getState().setFilter('review');
    expect(visibleIds(useLibrary.getState())).toEqual([2]);
    useLibrary.getState().setFilter('all');
    useLibrary.getState().setQuery('gam');
    expect(visibleIds(useLibrary.getState())).toEqual([3]);
  });

  it('N jumps to the next row that needs review and wraps', () => {
    const lib = useLibrary.getState();
    lib.add([1, 2, 3, 4, 5].map((i) => entry(i)));
    for (const i of [2, 4]) {
      lib.applyEvent(analysed(i, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    }
    lib.select(1);
    expect(useLibrary.getState().selectNextReview()).toBe(2);
    expect(useLibrary.getState().selectNextReview()).toBe(4);
    expect(useLibrary.getState().selectNextReview()).toBe(2);
  });

  it('arrows move the selection within the visible rows', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2), entry(3)]);
    expect(useLibrary.getState().selected).toBe(1);
    useLibrary.getState().moveSelection(1);
    useLibrary.getState().moveSelection(5);
    expect(useLibrary.getState().selected).toBe(3);
    useLibrary.getState().moveSelection(-1);
    expect(useLibrary.getState().selected).toBe(2);
  });

  it('new plans move rows between Analysed and Needs review without re-analysis', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2)]);
    lib.applyEvent(analysed(1));
    lib.applyReplan({
      revision: 1,
      plans: [
        { fileId: 1, plan: plan({ status: 'needsReview', review: [{ type: 'outsideBpmRange' }] }) },
        { fileId: 2, plan: plan() },
      ],
    });
    expect(useLibrary.getState().rows[1]?.state).toBe('needsReview');
    expect(useLibrary.getState().rows[2]?.state).toBe('queued');
  });

  it('marks failed, cancelled and other-range rows for Analyse', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2), entry(3)]);
    lib.queue([1, 2, 3], [70, 180]);
    lib.applyEvent(analysed(1));
    lib.applyEvent({ type: 'cancelled', jobId: 1, fileId: 2 });
    lib.applyEvent(analysed(3));
    expect(staleIds(useLibrary.getState(), [70, 180])).toEqual([2]);
    expect(staleIds(useLibrary.getState(), [80, 160])).toEqual([1, 2, 3]);
  });

  it('a job that ends early leaves no row waiting', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1)]);
    lib.queue([1], [70, 180]);
    lib.applyEvent({ type: 'aborted', jobId: 1, error: { kind: 'modelUnavailable', message: 'no model', fileId: null } });
    lib.applyEvent({ type: 'finished', jobId: 1, cancelled: false });
    expect(useLibrary.getState().rows[1]?.state).toBe('cancelled');
    expect(useLibrary.getState().aborted?.kind).toBe('modelUnavailable');
  });

  it('a replan newer than an analysed event keeps its plan', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1)]);
    lib.queue([1], [70, 180]);
    // The user switched to Streaming (revision 2) while row 1's DJ plan (revision 1) was in flight.
    const streaming = plan({ measured: -9.5, gain: { type: 'gain', gainDb: -4.5, shortByLu: 0, truePeakAfter: -4.8 } });
    lib.applyReplan({ revision: 2, plans: [{ fileId: 1, plan: streaming }] });
    expect(useLibrary.getState().rows[1]?.state).toBe('queued');
    lib.applyEvent({ ...analysed(1), revision: 1 } as never);
    const row = useLibrary.getState().rows[1];
    expect(row?.plan).toEqual(streaming);
    expect(row?.planRevision).toBe(2);
    expect(row?.analysis).toBeDefined();
    // An older replan never overwrites a newer plan either.
    useLibrary.getState().applyReplan({ revision: 1, plans: [{ fileId: 1, plan: plan() }] });
    expect(useLibrary.getState().rows[1]?.plan).toEqual(streaming);
  });

  it('the last batch counts only its own rows and leaves other rows waiting', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2), entry(3)]);
    lib.queue([1], [70, 180]);
    lib.applyEvent(analysed(1));
    lib.applyEvent({ type: 'finished', jobId: 1, cancelled: false });
    lib.queue([2], [70, 180]);
    lib.applyEvent({ type: 'failed', jobId: 2, fileId: 2, error: { kind: 'io', message: 'gone', fileId: 2 } });
    lib.applyEvent({ type: 'finished', jobId: 2, cancelled: false });
    const s = useLibrary.getState();
    expect(s.lastBatch).toMatchObject({ analysed: 0, failed: 1 });
    expect(s.rows[3]?.state).toBe('queued');
    expect(s.rows[1]?.state).toBe('analysed');
  });

  it('N only visits rows the filter shows', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1, 'Alpha'), entry(2, 'Beta'), entry(3, 'Alpine')]);
    for (const i of [1, 2, 3]) {
      lib.applyEvent(analysed(i, plan({ status: 'needsReview', review: [{ type: 'drifts' }] })));
    }
    useLibrary.getState().setQuery('alp');
    useLibrary.getState().select(1);
    expect(useLibrary.getState().selectNextReview()).toBe(3);
    expect(useLibrary.getState().selectNextReview()).toBe(1);
  });

  it('restores the table from the engine after a reload', () => {
    useLibrary.getState().restore(
      {
        revision: 4,
        rows: [
          { entry: entry(7), row: analysis(), plan: plan({ status: 'needsReview', review: [{ type: 'drifts' }] }) },
          { entry: entry(8), row: null, plan: null },
        ],
      },
      [70, 180],
    );
    const s = useLibrary.getState();
    expect(s.order).toEqual([7, 8]);
    expect(s.rows[7]).toMatchObject({ state: 'needsReview', planRevision: 4 });
    expect(s.rows[8]?.state).toBe('cancelled');
    expect(staleIds(s, [70, 180])).toEqual([8]);
  });
});
