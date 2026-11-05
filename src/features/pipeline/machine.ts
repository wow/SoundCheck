import { assign, setup } from 'xstate';
import type { ExportPreviewRow, ExportRequest } from '@/lib/ipc';

/**
 * The job pipeline: at most one job at a time, an analysis or an export. Files added while a job
 * runs wait in `pending` and start as the next analysis when it finishes. Cancelling an analysis
 * drops the files waiting at that moment; files added while the cancelled job winds down still
 * run next. Cancelling an export keeps them, since it was the export that was stopped.
 *
 * `windingDown` is a window that reloaded while a job ran: the new page cannot hear that job, so it
 * starts nothing until the session says the job has ended.
 *
 * The side effects are provided by the caller: `startJob` and `startExport` (start a job, then send
 * `STARTED` with its id and `FINISHED` when its last event arrives), `cancelJob`, and `watchSession`
 * (send `WOUND_DOWN` once no job is left).
 */
export interface PipelineContext {
  jobId: number | null;
  pending: number[];
  /** The export to start, and its confirm step's rows. */
  exportJob: ExportJob | null;
}

export interface ExportJob {
  request: ExportRequest;
  planned: ExportPreviewRow[];
}

export type PipelineEvent =
  | { type: 'ANALYSE'; fileIds: number[] }
  | { type: 'EXPORT'; job: ExportJob }
  | { type: 'STARTED'; jobId: number }
  | { type: 'CANCEL' }
  | { type: 'FINISHED' }
  | { type: 'WIND_DOWN' }
  | { type: 'WOUND_DOWN' };

function queued(context: PipelineContext, fileIds: number[]): number[] {
  return [...context.pending, ...fileIds.filter((id) => !context.pending.includes(id))];
}

export const pipelineMachine = setup({
  types: {
    context: {} as PipelineContext,
    events: {} as PipelineEvent,
  },
  actions: {
    startJob: (_, _params: { fileIds: number[] }) => {},
    startExport: (_, _params: { job: ExportJob | null }) => {},
    cancelJob: (_, _params: { jobId: number | null }) => {},
    watchSession: () => {},
    queueFiles: assign({
      pending: ({ context, event }) =>
        event.type === 'ANALYSE' ? queued(context, event.fileIds) : context.pending,
    }),
    jobStarted: assign({
      jobId: ({ context, event }) => (event.type === 'STARTED' ? event.jobId : context.jobId),
    }),
  },
  guards: {
    hasPending: ({ context }) => context.pending.length > 0,
  },
}).createMachine({
  id: 'pipeline',
  context: { jobId: null, pending: [], exportJob: null },
  initial: 'idle',
  on: {
    // From anywhere: only the first read after a reload can send it, and then nothing runs yet.
    WIND_DOWN: { target: '.windingDown' },
  },
  states: {
    idle: {
      on: {
        ANALYSE: {
          guard: ({ event }) => event.fileIds.length > 0,
          target: 'analysing',
          actions: assign({ pending: ({ event }) => event.fileIds }),
        },
        EXPORT: {
          guard: ({ event }) => event.job.request.fileIds.length > 0,
          target: 'exporting',
          actions: assign({ exportJob: ({ event }) => event.job }),
        },
      },
    },
    analysing: {
      entry: [
        { type: 'startJob', params: ({ context }) => ({ fileIds: context.pending }) },
        assign({ pending: [], jobId: null }),
      ],
      on: {
        STARTED: { actions: 'jobStarted' },
        ANALYSE: { actions: 'queueFiles' },
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
        ANALYSE: { actions: 'queueFiles' },
        // The job id can arrive after Cancel was pressed; cancel it then.
        STARTED: {
          actions: [
            'jobStarted',
            { type: 'cancelJob', params: ({ event }) => ({ jobId: event.jobId }) },
          ],
        },
        FINISHED: [
          { guard: 'hasPending', target: 'analysing' },
          { target: 'idle', actions: assign({ jobId: null }) },
        ],
      },
    },
    exporting: {
      entry: [
        {
          type: 'startExport',
          params: ({ context }) => ({ job: context.exportJob }),
        },
        assign({ jobId: null, exportJob: null }),
      ],
      on: {
        STARTED: { actions: 'jobStarted' },
        ANALYSE: { actions: 'queueFiles' },
        CANCEL: { target: 'cancellingExport' },
        FINISHED: [
          { guard: 'hasPending', target: 'analysing', actions: assign({ jobId: null }) },
          { target: 'idle', actions: assign({ jobId: null }) },
        ],
      },
    },
    cancellingExport: {
      entry: { type: 'cancelJob', params: ({ context }) => ({ jobId: context.jobId }) },
      on: {
        ANALYSE: { actions: 'queueFiles' },
        STARTED: {
          actions: [
            'jobStarted',
            { type: 'cancelJob', params: ({ event }) => ({ jobId: event.jobId }) },
          ],
        },
        FINISHED: [
          { guard: 'hasPending', target: 'analysing', actions: assign({ jobId: null }) },
          { target: 'idle', actions: assign({ jobId: null }) },
        ],
      },
    },
    windingDown: {
      entry: 'watchSession',
      on: {
        ANALYSE: { actions: 'queueFiles' },
        WOUND_DOWN: [{ guard: 'hasPending', target: 'analysing' }, { target: 'idle' }],
      },
    },
  },
});
