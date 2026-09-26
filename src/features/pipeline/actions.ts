import { createActor, type Actor } from 'xstate';
import { analyze, calibrationTarget, cancelJob, clearSession, expandPaths } from '@/lib/ipc';
import { ask } from '@/lib/platform';
import { useLibrary, staleIds } from '@/state/library';
import { analysisSettings, useSettings } from '@/state/settings';
import { pipelineMachine } from './machine';

/** The one pipeline of the app, wired to the engine and the library store. */
function createPipeline(): Actor<typeof pipelineMachine> {
  const actor: Actor<typeof pipelineMachine> = createActor(
    pipelineMachine.provide({
      actions: {
        startJob: (_, { fileIds }) => {
          const settings = useSettings.getState();
          const analysis = analysisSettings(settings);
          const library = useLibrary.getState();
          library.queue(fileIds, analysis.bpmRange);
          analyze({ fileIds, analysis }, (event) => {
            useLibrary.getState().applyEvent(event);
            if (event.type === 'finished') actor.send({ type: 'FINISHED' });
          })
            .then((jobId) => actor.send({ type: 'STARTED', jobId }))
            .catch((err: unknown) => {
              const message = err instanceof Error ? err.message : String(err);
              const lib = useLibrary.getState();
              lib.applyEvent({
                type: 'aborted',
                jobId: 0,
                error: { kind: 'internal', message, fileId: null },
              });
              lib.applyEvent({ type: 'finished', jobId: 0, cancelled: false });
              actor.send({ type: 'FINISHED' });
            });
        },
        cancelJob: (_, { jobId }) => {
          if (jobId !== null) void cancelJob(jobId);
        },
      },
    }),
  );
  actor.start();
  return actor;
}

let singleton: Actor<typeof pipelineMachine> | null = null;

/** The app's pipeline actor, created on first use. */
export function pipeline(): Actor<typeof pipelineMachine> {
  singleton ??= createPipeline();
  return singleton;
}

/** Adds dropped or chosen paths and starts analysing the new rows. */
export async function addPaths(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  const entries = await expandPaths(paths);
  if (entries.length === 0) return;
  useLibrary.getState().add(entries);
  pipeline().send({ type: 'ANALYSE', fileIds: entries.map((e) => e.fileId) });
}

/** Re-runs failed, cancelled and waiting rows, and rows analysed with another BPM range. */
export function analyseStale(): void {
  const ids = staleIds(useLibrary.getState(), useSettings.getState().bpmRange);
  pipeline().send({ type: 'ANALYSE', fileIds: ids });
}

/** Empties the track list; the files on disk are not touched and their analyses stay cached. */
export async function clearList(): Promise<void> {
  pipeline().send({ type: 'CANCEL' });
  await clearSession();
  useLibrary.getState().clear();
}

/** Stops the running job. */
export function cancelAnalysis(): void {
  pipeline().send({ type: 'CANCEL' });
}

/** Sets the DJ target to the median S-P95 of the analysed rows, after asking. */
export async function calibrateFromLibrary(): Promise<void> {
  const median = await calibrationTarget();
  const lib = useLibrary.getState();
  const n = Object.values(lib.rows).filter((r) => r.analysis?.shortTermP95 != null).length;
  if (median === null || n === 0) return;
  const value = median.toFixed(1);
  const ok = await ask(
    `Set the DJ target to ${value} LUFS, the median S-P95 of ${n} analysed tracks? Every track will then be levelled to it.`,
    'Calibrate from my library',
    `Use ${value} LUFS`,
  );
  if (ok) useSettings.getState().calibrate(median);
}
