import { confirm, open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';

/** Asks for folders; resolves to the chosen paths (none when cancelled). */
export async function chooseFolders(): Promise<string[]> {
  const picked = await open({ directory: true, multiple: true, title: 'Add folders' });
  return picked ?? [];
}

/** Asks for audio files; resolves to the chosen paths (none when cancelled). */
export async function chooseFiles(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    title: 'Add tracks',
    filters: [
      {
        name: 'Audio',
        extensions: ['wav', 'wave', 'aif', 'aiff', 'aifc', 'flac', 'mp3', 'm4a', 'mp4', 'aac', 'ogg', 'oga', 'opus'],
      },
    ],
  });
  return picked ?? [];
}

/** A yes/no question in a native sheet. */
export function ask(message: string, title: string, okLabel: string): Promise<boolean> {
  return confirm(message, { title, okLabel, cancelLabel: 'Cancel' });
}

export type DropState = 'over' | 'none';

/**
 * Calls `onPaths` with the paths dropped on the window and `onHover` as a drag enters or leaves.
 * Returns an unsubscribe function; outside the desktop shell (tests, the browser harness) it does
 * nothing.
 */
export function listenForDrops(
  onPaths: (paths: string[]) => void,
  onHover: (state: DropState) => void,
): () => void {
  let unlisten: (() => void) | null = null;
  let stopped = false;
  try {
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === 'enter' || p.type === 'over') onHover('over');
        else if (p.type === 'leave') onHover('none');
        else {
          onHover('none');
          if (p.paths.length > 0) onPaths(p.paths);
        }
      })
      .then((fn) => {
        if (stopped) fn();
        else unlisten = fn;
      })
      .catch(() => {});
  } catch {
    // Not inside the desktop shell.
  }
  return () => {
    stopped = true;
    unlisten?.();
  };
}
