import { createActor } from 'xstate';
import { pipelineMachine, type PipelineEvent } from './machine';

function run() {
  const started: number[][] = [];
  const exports: number[][] = [];
  const cancelled: (number | null)[] = [];
  let watched = 0;
  const actor = createActor(
    pipelineMachine.provide({
      actions: {
        startJob: (_, { fileIds }) => void started.push(fileIds),
        startExport: (_, { job }) => void exports.push(job?.request.fileIds ?? []),
        cancelJob: (_, { jobId }) => void cancelled.push(jobId),
        watchSession: () => {
          watched += 1;
        },
      },
    }),
  );
  actor.start();
  return { actor, started, exports, cancelled, watched: () => watched };
}

function exportEvent(fileIds: number[]): PipelineEvent {
  return {
    type: 'EXPORT',
    job: {
      request: {
        fileIds,
        settings: {
          batchMode: 'prepare',
          place: 'inPlace',
          depth: null,
          gridOnly: false,
          tbpm: true,
          xml: true,
          leadMs: 5,
        },
        outDir: null,
      },
      planned: [],
    },
  };
}

describe('pipeline machine', () => {
  it('analyses, then goes idle when the job finishes', () => {
    const { actor, started } = run();
    actor.send({ type: 'ANALYSE', fileIds: [1, 2] });
    expect(actor.getSnapshot().value).toBe('analysing');
    expect(started).toEqual([[1, 2]]);
    actor.send({ type: 'STARTED', jobId: 5 });
    expect(actor.getSnapshot().context.jobId).toBe(5);
    actor.send({ type: 'FINISHED' });
    expect(actor.getSnapshot().value).toBe('idle');
  });

  it('files added during a job start as the next job', () => {
    const { actor, started } = run();
    actor.send({ type: 'ANALYSE', fileIds: [1] });
    actor.send({ type: 'ANALYSE', fileIds: [2, 3] });
    actor.send({ type: 'ANALYSE', fileIds: [3, 4] });
    actor.send({ type: 'FINISHED' });
    expect(started).toEqual([[1], [2, 3, 4]]);
    expect(actor.getSnapshot().value).toBe('analysing');
    actor.send({ type: 'FINISHED' });
    expect(actor.getSnapshot().value).toBe('idle');
  });

  it('cancel stops the job and drops what was waiting', () => {
    const { actor, started, cancelled } = run();
    actor.send({ type: 'ANALYSE', fileIds: [1] });
    actor.send({ type: 'STARTED', jobId: 9 });
    actor.send({ type: 'ANALYSE', fileIds: [2] });
    actor.send({ type: 'CANCEL' });
    expect(cancelled).toEqual([9]);
    actor.send({ type: 'FINISHED' });
    expect(actor.getSnapshot().value).toBe('idle');
    expect(started).toEqual([[1]]);
  });

  it('a cancel before the job id arrives cancels it on arrival', () => {
    const { actor, cancelled } = run();
    actor.send({ type: 'ANALYSE', fileIds: [1] });
    actor.send({ type: 'CANCEL' });
    actor.send({ type: 'STARTED', jobId: 3 });
    expect(cancelled).toEqual([null, 3]);
  });

  it('ignores an empty request', () => {
    const { actor, started } = run();
    actor.send({ type: 'ANALYSE', fileIds: [] });
    expect(actor.getSnapshot().value).toBe('idle');
    expect(started).toEqual([]);
  });

  it('files dropped while a cancel winds down start as the next job', () => {
    const { actor, started } = run();
    actor.send({ type: 'ANALYSE', fileIds: [1] });
    actor.send({ type: 'STARTED', jobId: 2 });
    actor.send({ type: 'CANCEL' });
    actor.send({ type: 'ANALYSE', fileIds: [5, 6] });
    actor.send({ type: 'FINISHED' });
    expect(actor.getSnapshot().value).toBe('analysing');
    expect(started).toEqual([[1], [5, 6]]);
  });

  describe('export', () => {
    it('runs the export, then goes idle when it finishes', () => {
      const { actor, exports, started } = run();
      actor.send(exportEvent([1, 2]));
      expect(actor.getSnapshot().value).toBe('exporting');
      expect(exports).toEqual([[1, 2]]);
      expect(started).toEqual([]);
      actor.send({ type: 'STARTED', jobId: 4 });
      expect(actor.getSnapshot().context.jobId).toBe(4);
      actor.send({ type: 'FINISHED' });
      expect(actor.getSnapshot().value).toBe('idle');
      expect(actor.getSnapshot().context.jobId).toBeNull();
    });

    it('ignores an export of nothing', () => {
      const { actor, exports } = run();
      actor.send(exportEvent([]));
      expect(actor.getSnapshot().value).toBe('idle');
      expect(exports).toEqual([]);
    });

    it('is one job at a time: no export starts during an analysis or another export', () => {
      const { actor, exports } = run();
      actor.send({ type: 'ANALYSE', fileIds: [1] });
      actor.send(exportEvent([1]));
      expect(actor.getSnapshot().value).toBe('analysing');
      expect(exports).toEqual([]);
      actor.send({ type: 'FINISHED' });
      actor.send(exportEvent([1]));
      actor.send(exportEvent([2]));
      expect(exports).toEqual([[1]]);
    });

    it('files added during an export wait and start as the next analysis', () => {
      const { actor, started } = run();
      actor.send(exportEvent([1]));
      actor.send({ type: 'ANALYSE', fileIds: [2] });
      actor.send({ type: 'ANALYSE', fileIds: [2, 3] });
      expect(started).toEqual([]);
      actor.send({ type: 'FINISHED' });
      expect(actor.getSnapshot().value).toBe('analysing');
      expect(started).toEqual([[2, 3]]);
    });

    it('cancel stops the export and keeps the files waiting, which the export did not start', () => {
      const { actor, cancelled, started } = run();
      actor.send(exportEvent([1]));
      actor.send({ type: 'STARTED', jobId: 8 });
      actor.send({ type: 'ANALYSE', fileIds: [5] });
      actor.send({ type: 'CANCEL' });
      expect(actor.getSnapshot().value).toBe('cancellingExport');
      expect(cancelled).toEqual([8]);
      actor.send({ type: 'FINISHED' });
      expect(started).toEqual([[5]]);
    });

    it('a cancel before the job id arrives cancels the export on arrival', () => {
      const { actor, cancelled } = run();
      actor.send(exportEvent([1]));
      actor.send({ type: 'CANCEL' });
      actor.send({ type: 'STARTED', jobId: 6 });
      expect(cancelled).toEqual([null, 6]);
      actor.send({ type: 'FINISHED' });
      expect(actor.getSnapshot().value).toBe('idle');
    });
  });

  describe('a reload during a job', () => {
    it('waits for the old job before starting anything', () => {
      const { actor, started, exports, watched } = run();
      actor.send({ type: 'WIND_DOWN' });
      expect(actor.getSnapshot().value).toBe('windingDown');
      expect(watched()).toBe(1);
      actor.send({ type: 'ANALYSE', fileIds: [3] });
      actor.send(exportEvent([3]));
      expect(started).toEqual([]);
      expect(exports).toEqual([]);
      actor.send({ type: 'WOUND_DOWN' });
      expect(actor.getSnapshot().value).toBe('analysing');
      expect(started).toEqual([[3]]);
    });

    it('goes idle when nothing waits', () => {
      const { actor } = run();
      actor.send({ type: 'WIND_DOWN' });
      actor.send({ type: 'WOUND_DOWN' });
      expect(actor.getSnapshot().value).toBe('idle');
    });
  });
});
