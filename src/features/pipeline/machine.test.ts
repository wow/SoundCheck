import { createActor } from 'xstate';
import { pipelineMachine } from './machine';

function run() {
  const started: number[][] = [];
  const cancelled: (number | null)[] = [];
  const actor = createActor(
    pipelineMachine.provide({
      actions: {
        startJob: (_, { fileIds }) => void started.push(fileIds),
        cancelJob: (_, { jobId }) => void cancelled.push(jobId),
      },
    }),
  );
  actor.start();
  return { actor, started, cancelled };
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
});
