import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ExportPreview, ExportRequest } from '@/lib/ipc';
import { pipeline } from '@/features/pipeline/actions';
import { useTrack } from '@/features/grid/store';
import { useLibrary } from '@/state/library';
import { usePanels } from '@/state/panels';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { analysed, entry, exportRow } from '@/test/fixtures';
import { ExportConfirm } from './ExportConfirm';
import { dismissExportConfirm, exportBlock, openExportConfirm, useExportFlow } from './flow';

const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();

function add(...ids: number[]) {
  useLibrary.getState().add(ids.map((id) => entry(id)));
  for (const id of ids) useLibrary.getState().applyEvent(analysed(id));
}

const cutRow = exportRow();
const skipRow = exportRow({
  stage: 'skipped',
  action: { type: 'skip', reason: { type: 'silent' } },
  positionsShift: false,
  notes: ['it has no audio above the -70 LUFS gate'],
});

function preview(rows: ExportPreview['rows']): ExportPreview {
  return { rows, backupFolder: '/Users/dj/Music/SoundCheck Backups', artefactsFolder: '/Users/dj/Music/SoundCheck/exports' };
}

/** Answers the confirm step with `answer`; `sent` holds the requests. */
function backend(answer: ExportPreview | { throws: unknown }) {
  const planned: ExportRequest[] = [];
  const exported: ExportRequest[] = [];
  mockIPC((cmd, args) => {
    const req = (args as { req: ExportRequest }).req;
    if (cmd === 'plan_export') {
      planned.push(req);
      if ('throws' in answer) throw answer.throws;
      return answer;
    }
    if (cmd === 'export') {
      exported.push(req);
      return 5;
    }
    return undefined;
  });
  return { planned, exported };
}

beforeEach(() => {
  useLibrary.setState(initialLibrary, true);
  useSettings.setState(initialSettings, true);
  useRecovery.setState({ running: false });
  useExportFlow.setState({ phase: 'closed', request: null, preview: null, error: null, leftOut: 0 });
  // A pipeline left running by a test before this one.
  pipeline().send({ type: 'FINISHED' });
});

describe('the confirm step', () => {
  it('asks the engine first and writes nothing until confirmed', async () => {
    add(1, 2);
    const { planned, exported } = backend(
      preview([
        { fileId: 1, row: cutRow, error: null },
        { fileId: 2, row: skipRow, error: null },
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByText('2 files: 1 gain, 1 cut, 1 skipped; backups in ~/Music/SoundCheck Backups');
    expect(planned[0]?.fileIds).toEqual([1, 2]);
    expect(exported).toEqual([]);
    expect(useLibrary.getState().rows[1]?.state).toBe('analysed');

    // One file to write; the skipped one still goes along, to get its row.
    await userEvent.click(screen.getByRole('button', { name: 'Export 1 file' }));
    await waitFor(() => expect(exported).toHaveLength(1));
    expect(exported[0]).toEqual(planned[0]);
    expect(useExportFlow.getState().phase).toBe('closed');
    // What it planned is what the rows show while they wait.
    expect(useLibrary.getState().rows[1]).toMatchObject({ state: 'exportQueued', export: cutRow });
  });

  it('says plainly that a Prepare cut shifts positions, and how far', async () => {
    add(1);
    backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    render(<ExportConfirm />);
    void openExportConfirm();
    const warning = await screen.findByText(/Positions shift/);
    expect(warning).toHaveTextContent('Positions shift on 1 cut file, by up to 0.21 s');
    expect(warning).toHaveTextContent('Library');
  });

  it('says nothing about positions when nothing is cut', async () => {
    add(1);
    backend(
      preview([
        {
          fileId: 1,
          row: exportRow({ action: { type: 'write', gainDb: -1, cut: { type: 'library' } }, positionsShift: false }),
          error: null,
        },
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Export 1 file' });
    expect(screen.queryByText(/Positions shift/)).toBeNull();
  });

  it('lists each skipped file with the row\'s own words', async () => {
    add(1, 2);
    backend(
      preview([
        { fileId: 1, row: cutRow, error: null },
        { fileId: 2, row: skipRow, error: null },
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    const list = await screen.findByRole('list', { name: 'File by file' });
    expect(within(list).getByText('Track 2')).toBeInTheDocument();
    expect(within(list).getByText(/Skipped: it has no audio above/)).toBeInTheDocument();
  });

  it('shows a file that could not be planned', async () => {
    add(1);
    backend(preview([{ fileId: 1, row: null, error: { kind: 'corrupt', message: 'truncated', fileId: 1 } }]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText(/Cannot be planned: truncated/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Nothing to export' })).toBeDisabled();
  });

  it('leaves out the rows not analysed, and says so', async () => {
    add(1);
    useLibrary.getState().add([entry(2)]);
    const { planned } = backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText('1 track not analysed is left out.')).toBeInTheDocument();
    expect(planned[0]?.fileIds).toEqual([1]);
  });

  it('a refusal shows the real copy, not [object Object], with no way to export', async () => {
    add(1);
    backend({ throws: { kind: 'busy', message: 'raw', fileId: 1 } });
    render(<ExportConfirm />);
    void openExportConfirm();
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Another job is working on one of these files');
    expect(alert).not.toHaveTextContent('[object');
    expect(screen.queryByRole('button', { name: /^Export/ })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(useExportFlow.getState().phase).toBe('closed');
  });

  it('Cancel and Esc write nothing, and a late answer lands nowhere', async () => {
    add(1);
    const { exported } = backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Export 1 file' });
    await userEvent.keyboard('{Escape}');
    expect(useExportFlow.getState().phase).toBe('closed');
    expect(exported).toEqual([]);

    const opening = openExportConfirm();
    dismissExportConfirm();
    await opening;
    expect(useExportFlow.getState()).toMatchObject({ phase: 'closed', preview: null });
  });

  it('sends the folder and the mode the Serato rule picked', async () => {
    const e = entry(1);
    useLibrary.getState().add([{ ...e, info: { ...e.info, serato: true } }]);
    useLibrary.getState().applyEvent(analysed(1));
    useSettings.setState({ place: 'folder', outDir: '/Out', depth: 16, xml: false });
    const { planned } = backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('dialog');
    await waitFor(() => expect(planned).toHaveLength(1));
    expect(planned[0]).toMatchObject({
      outDir: '/Out',
      settings: { batchMode: 'library', place: 'folder', depth: 16, xml: false, tbpm: false },
    });
  });

  it('gives way to nothing during recovery', async () => {
    add(1);
    const { planned } = backend(preview([]));
    useRecovery.setState({ running: true });
    await openExportConfirm();
    expect(useExportFlow.getState().phase).toBe('closed');
    expect(planned).toEqual([]);
  });

  it('closes the settings drawer of a narrow window: one modal panel at a time', async () => {
    add(1);
    backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    usePanels.getState().setSettings(true);
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('dialog');
    expect(usePanels.getState().settings).toBe(false);
  });

  it('closes the grid view of a file about to be replaced, and says where it went', async () => {
    add(1);
    backend(preview([{ fileId: 1, row: cutRow, error: null }]));
    const close = vi.fn(async () => {});
    useTrack.setState({ fileId: 1, close });
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Export 1 file' }));
    expect(close).toHaveBeenCalled();
    expect(useLibrary.getState().info).toMatch(/grid view closed: Track 1 is being exported/);
    useTrack.setState({ fileId: null });
  });
});

describe('why Export cannot be pressed', () => {
  const gate = {
    exportable: 2,
    hasRows: true,
    anyExported: false,
    busy: false,
    reverting: false,
    xmlWriting: false,
    recoveryRunning: false,
    needsFolder: false,
  };

  it('has a reason for each case, and none when it can', () => {
    expect(exportBlock(gate)).toBeNull();
    expect(exportBlock({ ...gate, recoveryRunning: true })).toMatch(/crash interrupted/);
    expect(exportBlock({ ...gate, busy: true })).toMatch(/job is running/);
    expect(exportBlock({ ...gate, reverting: true })).toMatch(/revert is running/);
    expect(exportBlock({ ...gate, xmlWriting: true })).toMatch(/XML/);
    expect(exportBlock({ ...gate, exportable: 0 })).toBe('Analyse the tracks first.');
    expect(exportBlock({ ...gate, exportable: 0, anyExported: true })).toMatch(/Revert a track, or save its grid/);
    expect(exportBlock({ ...gate, needsFolder: true })).toMatch(/Choose a folder/);
  });
});
