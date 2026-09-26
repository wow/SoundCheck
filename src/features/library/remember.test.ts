import { waitFor } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { FileEntry } from '@/lib/ipc';
import { loadTrackList, saveTrackList } from '@/lib/persist';
import { useLibrary } from '@/state/library';
import { analysis, entry, plan } from '@/test/fixtures';
import { startTrackListPersistence } from './remember';

vi.mock('@/lib/persist', () => ({
  loadTrackList: vi.fn(async () => [] as string[]),
  saveTrackList: vi.fn(async () => {}),
  loadSettings: vi.fn(async () => ({})),
  saveSettings: vi.fn(async () => {}),
}));

const initial = useLibrary.getState();
beforeEach(() => {
  useLibrary.setState(initial, true);
  vi.mocked(loadTrackList).mockReset().mockResolvedValue([]);
  vi.mocked(saveTrackList).mockReset().mockResolvedValue();
});

describe('remembering the track list', () => {
  it('adds the saved paths again after a restart and keeps saving the list', async () => {
    vi.mocked(loadTrackList).mockResolvedValue(['/music/A.flac', '/music/B.flac']);
    let expanded: string[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'restore_session') return { revision: 0, rows: [] };
      if (cmd === 'expand_paths') {
        expanded = (args as { paths: string[] }).paths;
        return expanded.map((p, i): FileEntry => ({ ...entry(i + 1), path: p }));
      }
      if (cmd === 'analyze') return 1;
      return null;
    });
    const stop = startTrackListPersistence();
    await waitFor(() => expect(useLibrary.getState().order).toEqual([1, 2]));
    expect(expanded).toEqual(['/music/A.flac', '/music/B.flac']);
    useLibrary.getState().add([{ ...entry(3), path: '/music/C.flac' }]);
    await waitFor(() =>
      expect(saveTrackList).toHaveBeenLastCalledWith(['/music/A.flac', '/music/B.flac', '/music/C.flac']),
    );
    stop();
  });

  it('after a window reload takes the rows from the engine instead', async () => {
    mockIPC((cmd) => {
      if (cmd === 'restore_session') {
        return { revision: 2, rows: [{ entry: entry(9), row: analysis(), plan: plan() }] };
      }
      return null;
    });
    const stop = startTrackListPersistence();
    await waitFor(() => expect(useLibrary.getState().order).toEqual([9]));
    expect(loadTrackList).not.toHaveBeenCalled();
    expect(useLibrary.getState().rows[9]?.state).toBe('analysed');
    stop();
  });

  it('a cleared list stays empty and a late job end reports nothing', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1), entry(2)]);
    lib.queue([1, 2], [70, 180]);
    lib.clear();
    useLibrary.getState().applyEvent({ type: 'finished', jobId: 1, cancelled: true });
    const s = useLibrary.getState();
    expect(s.order).toEqual([]);
    expect(s.lastBatch).toBeNull();
  });
});
