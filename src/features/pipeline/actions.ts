import { createActor, type Actor } from 'xstate';
import {
  analyze,
  calibrationTarget,
  cancelJob,
  clearSession,
  expandPaths,
  exportFiles,
  ipcErrorOf,
  restoreSession,
  type JobEvent,
} from '@/lib/ipc';
import { ask } from '@/lib/platform';
import { staleIds, useLibrary, writtenWithoutAnalysis } from '@/state/library';
import { analysisSettings, useSettings } from '@/state/settings';
import { pipelineMachine } from './machine';

/** How often a reloaded window reads the session while the job it cannot hear winds down. */
export const WIND_DOWN_POLL_MS = 500;

/** How long it waits for that job before saying something is wrong; a cancel takes seconds. */
export const WIND_DOWN_LIMIT_MS = 60_000;

/** The one pipeline of the app, wired to the engine and the library store. */
function createPipeline(): Actor<typeof pipelineMachine> {
  /** A job that never started: what stopped it shows, and its rows end as they were. */
  const refused = (err: unknown) => {
    const lib = useLibrary.getState();
    lib.applyEvent({ type: 'aborted', jobId: 0, error: ipcErrorOf(err) });
    lib.applyEvent({ type: 'finished', jobId: 0, cancelled: false });
    actor.send({ type: 'FINISHED' });
  };
  const onExportEvent = (event: JobEvent) => {
    useLibrary.getState().applyEvent(event);
    // Replaced in place and not analysed: what the row showed is gone, so it is analysed again.
    if (event.type === 'exportDone' && event.inPlace && !event.update) {
      actor.send({ type: 'ANALYSE', fileIds: [event.fileId] });
    }
    if (event.type === 'finished') actor.send({ type: 'FINISHED' });
  };
  const watchSession = async () => {
    const limit = Date.now() + WIND_DOWN_LIMIT_MS;
    try {
      for (;;) {
        await new Promise((go) => setTimeout(go, WIND_DOWN_POLL_MS));
        const snapshot = await restoreSession();
        useLibrary.getState().restore(snapshot);
        if (snapshot.runningJobs.length === 0) break;
        if (Date.now() >= limit) {
          useLibrary
            .getState()
            .setNotice('A job from before the window reloaded has not stopped. Restart SoundCheck to be sure of its files.');
          break;
        }
      }
    } catch {
      // The session cannot be read: nothing more can be known about that job.
    }
    useLibrary.getState().windDownEnded();
    analyseWritten();
    actor.send({ type: 'WOUND_DOWN' });
  };
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
            .catch(refused);
        },
        startExport: (_, { job }) => {
          if (!job) return;
          useLibrary.getState().beginExport(job.request.fileIds, job.planned);
          exportFiles(job.request, onExportEvent)
            .then((jobId) => actor.send({ type: 'STARTED', jobId }))
            .catch(refused);
        },
        cancelJob: (_, { jobId }) => {
          if (jobId !== null) void cancelJob(jobId);
        },
        watchSession: () => void watchSession(),
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

/** Analyses rows written in place whose own analysis did not finish, as a live export does. */
export function analyseWritten(): void {
  const { rows, order } = useLibrary.getState();
  const fileIds = writtenWithoutAnalysis(rows, order);
  if (fileIds.length > 0) pipeline().send({ type: 'ANALYSE', fileIds });
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
export function cancelRunningJob(): void {
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
