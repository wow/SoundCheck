import { assign, setup } from 'xstate';

/**
 * The analysis pipeline: at most one job at a time. Files added while a job runs wait in
 * `pending` and start as the next job when it finishes. Cancelling drops the files waiting at
 * that moment; files added while the cancelled job winds down still run next.
 *
 * The side effects are provided by the caller: `startJob` (start a job for `pending`, then send
 * `STARTED` with its id and `FINISHED` when its last event arrives) and `cancelJob`.
 */
export interface PipelineContext {
  jobId: number | null;
  pending: number[];
}

export type PipelineEvent =
  | { type: 'ANALYSE'; fileIds: number[] }
  | { type: 'STARTED'; jobId: number }
  | { type: 'CANCEL' }
  | { type: 'FINISHED' };

export const pipelineMachine = setup({
  types: {
    context: {} as PipelineContext,
    events: {} as PipelineEvent,
  },
  actions: {
    startJob: (_, _params: { fileIds: number[] }) => {},
    cancelJob: (_, _params: { jobId: number | null }) => {},
  },
  guards: {
    hasPending: ({ context }) => context.pending.length > 0,
  },
}).createMachine({
  id: 'pipeline',
  context: { jobId: null, pending: [] },
  initial: 'idle',
  states: {
    idle: {
      on: {
        ANALYSE: {
          guard: ({ event }) => event.fileIds.length > 0,
          target: 'analysing',
          actions: assign({ pending: ({ event }) => event.fileIds }),
        },
      },
    },
    analysing: {
      entry: [
        { type: 'startJob', params: ({ context }) => ({ fileIds: context.pending }) },
        assign({ pending: [], jobId: null }),
      ],
      on: {
        STARTED: { actions: assign({ jobId: ({ event }) => event.jobId }) },
        ANALYSE: {
          actions: assign({
            pending: ({ context, event }) => [
              ...context.pending,
              ...event.fileIds.filter((id) => !context.pending.includes(id)),
            ],
          }),
        },
        CANCEL: { target: 'cancelling' },
        FINISHED: [
          { guard: 'hasPending', target: 'analysing', reenter: true },
          { target: 'idle', actions: assign({ jobId: null }) },
        ],
      },
    },
    cancelling: {
      entry: [
        { type: 'cancelJob', params: ({ context }) => ({ jobId: context.jobId }) },
        assign({ pending: [] }),
      ],
      on: {
        // Files added while the old job winds down start as the next job.
        ANALYSE: {
          actions: assign({
            pending: ({ context, event }) => [
              ...context.pending,
              ...event.fileIds.filter((id) => !context.pending.includes(id)),
            ],
          }),
        },
        // The job id can arrive after Cancel was pressed; cancel it then.
        STARTED: {
          actions: [
            assign({ jobId: ({ event }) => event.jobId }),
            { type: 'cancelJob', params: ({ event }) => ({ jobId: event.jobId }) },
          ],
        },
        FINISHED: [
          { guard: 'hasPending', target: 'analysing' },
          { target: 'idle', actions: assign({ jobId: null }) },
        ],
      },
    },
  },
});
