import { restoreSession } from '@/lib/ipc';
import { loadTrackList, saveTrackList } from '@/lib/persist';
import { useLibrary } from '@/state/library';
import { useSettings } from '@/state/settings';
import { addPaths, pipeline } from '@/features/pipeline/actions';

/**
 * Brings the track list back when the app starts and keeps it saved afterwards.
 *
 * After a window reload the engine still holds the rows, so they come from it. After a restart
 * the saved file paths are added again; their analyses come from the disk cache, and a file that
 * was moved or deleted shows as an error row. A job still winding down when the window reloaded
 * cannot be heard from this page, so the pipeline waits it out. Returns a stop function.
 */
export function startTrackListPersistence(): () => void {
  let stopped = false;
  let unsubscribe = () => {};
  const watch = () => {
    let last = useLibrary.getState().order;
    unsubscribe = useLibrary.subscribe((s) => {
      if (s.order === last) return;
      last = s.order;
      void saveTrackList(s.order.map((id) => s.rows[id]?.entry.path).filter((p): p is string => !!p));
    });
  };
  void (async () => {
    try {
      const snapshot = await restoreSession();
      if (stopped) return;
      if (snapshot?.runningJobs?.length) pipeline().send({ type: 'WIND_DOWN' });
      if (snapshot?.rows?.length) {
        useLibrary.getState().restore(snapshot, useSettings.getState().bpmRange);
      } else {
        const paths = await loadTrackList();
        if (stopped) return;
        if (paths.length > 0) await addPaths(paths);
      }
    } catch {
      // Nothing to bring back; the list starts empty.
    }
    if (!stopped) watch();
  })();
  return () => {
    stopped = true;
    unsubscribe();
  };
}
