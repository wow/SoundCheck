import { mockIPC } from '@tauri-apps/api/mocks';
import type { RevertResult } from '@/lib/ipc';
import { useTrack } from '@/features/grid/store';
import { openInGridView } from '@/features/grid/actions';
import { pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { analysed, analysis, artefacts, doneRow, entry, plan } from '@/test/fixtures';
import { reveal, revert, writeXmlOfAnalysed } from './actions';

const initial = useLibrary.getState();
const lib = () => useLibrary.getState();

function doneRowOn(id: number) {
  lib().add([entry(id)]);
  lib().applyEvent(analysed(id));
  lib().beginExport([id], []);
  lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: id, row: doneRow(), inPlace: true, update: null });
  lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
  // Back to analysed, with its export.
  lib().queue([id], [70, 180]);
  lib().applyEvent(analysed(id));
  lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
}

const result: RevertResult = {
  entry: entry(1),
  update: { fileId: 1, row: analysis(), plan: plan(), revision: 0, exportStale: false },
  backup: '/backups/a',
  earlierChanges: 0,
  notes: [],
};

beforeEach(() => {
  useLibrary.setState(initial, true);
  useRecovery.setState({ running: false });
  useTrack.setState({ fileId: null });
});

describe('revert', () => {
  it('shows it on the row while it runs, and leaves the rest of the app alone', async () => {
    doneRowOn(1);
    lib().add([entry(2)]);
    let finish: (r: RevertResult) => void = () => {};
    mockIPC((cmd) => (cmd === 'revert_file' ? new Promise<RevertResult>((go) => (finish = go)) : undefined));
    const running = revert(1);
    expect(lib().rows[1]?.reverting).toBe(true);
    expect(pipeline().getSnapshot().value).toBe('idle');
    // Another row stays free.
    expect(lib().rows[2]?.reverting).toBeUndefined();
    finish(result);
    await running;
    expect(lib().rows[1]).toMatchObject({ reverting: false, state: 'analysed' });
    expect(lib().rows[1]?.export).toBeUndefined();
  });

  it('a refusal shows the real copy on the row, not [object Object]', async () => {
    doneRowOn(1);
    mockIPC(() => {
      throw { kind: 'busy', message: 'being exported', fileId: 1 };
    });
    await revert(1);
    expect(lib().rows[1]).toMatchObject({ reverting: false });
    expect(lib().rows[1]?.revertError?.kind).toBe('busy');
  });

  it('does nothing for a row of a running batch, without asking the engine', async () => {
    doneRowOn(1);
    const calls: string[] = [];
    mockIPC((cmd) => void calls.push(cmd));
    useLibrary.setState({ locked: new Set([1]) });
    await revert(1);
    expect(calls).not.toContain('revert_file');
  });

  it('asks for a new analysis when the original could not be analysed', async () => {
    doneRowOn(1);
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === 'revert_file') return { ...result, update: null, notes: ['not analysed'] };
      if (cmd === 'analyze') return 4;
      return undefined;
    });
    await revert(1);
    expect(calls).toContain('analyze');
    expect(lib().rows[1]?.revertNotes).toEqual(['not analysed']);
    pipeline().send({ type: 'FINISHED' });
  });

  it('closes the grid view of that file and says where it went', async () => {
    doneRowOn(1);
    const close = vi.fn(async () => {});
    useTrack.setState({ fileId: 1, close });
    mockIPC((cmd) => (cmd === 'revert_file' ? result : undefined));
    await revert(1);
    expect(close).toHaveBeenCalled();
    expect(lib().info).toMatch(/grid view closed: Track 1 is being reverted/);
  });
});

describe('opening a row', () => {
  it('stays closed on a row of a running batch or one being reverted, keeping the selection', () => {
    const open = vi.fn(async () => {});
    useTrack.setState({ open });
    lib().add([entry(1), entry(2)]);
    lib().applyEvent(analysed(1));
    lib().applyEvent(analysed(2));
    lib().beginExport([1], []);
    openInGridView(1);
    expect(open).not.toHaveBeenCalled();
    expect(lib().selected).toBe(1);
    openInGridView(2);
    expect(open).toHaveBeenCalledWith(2);
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: true });
    open.mockClear();
    lib().setReverting(2);
    openInGridView(2);
    expect(open).not.toHaveBeenCalled();
  });
});

describe('the rekordbox XML button', () => {
  it('lists every analysed row, exported ones too, and shows what it wrote', async () => {
    doneRowOn(1);
    lib().add([entry(2), entry(3)]);
    lib().applyEvent(analysed(2));
    let asked: unknown;
    mockIPC((cmd, args) => {
      if (cmd === 'write_rekordbox_xml') {
        asked = (args as { req: unknown }).req;
        return { artefacts: artefacts({ listed: 0 }), notListed: [{ fileId: 2, why: 'no grid' }] };
      }
      return undefined;
    });
    await writeXmlOfAnalysed();
    expect(asked).toEqual({ fileIds: [1, 2], out: null });
    expect(lib()).toMatchObject({ xmlWriting: false, xmlLeftOut: 1 });
    expect(lib().artefacts?.listed).toBe(0);
  });

  it('a failure lands where the artefacts would, with its kind', async () => {
    lib().add([entry(1)]);
    lib().applyEvent(analysed(1));
    mockIPC(() => {
      throw { kind: 'recoveryRunning', message: 'raw', fileId: null };
    });
    await writeXmlOfAnalysed();
    expect(lib()).toMatchObject({ xmlWriting: false, artefacts: null });
    expect(lib().artefactsFailed?.kind).toBe('recoveryRunning');
  });
});

describe('showing a file in the Finder', () => {
  it('asks the opener, and says so when it cannot', async () => {
    const shown: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'plugin:opener|reveal_item_in_dir') {
        shown.push(args);
        throw { kind: 'io', message: 'gone', fileId: null };
      }
      return undefined;
    });
    reveal('/exports/a.xml');
    await vi.waitFor(() => expect(lib().notice).toMatch(/Could not show it in the Finder: Gone/));
    expect(shown).toHaveLength(1);
  });
});
