import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ExportPreview, ExportRequest, JobEvent } from '@/lib/ipc';
import { analyseStale, pipeline } from '@/features/pipeline/actions';
import { useTrack } from '@/features/grid/store';
import { Rail } from '@/features/settings/Rail';
import { startSettingsSync } from '@/features/settings/sync';
import { staleIds, useLibrary } from '@/state/library';
import { usePanels } from '@/state/panels';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { analysed, doneRow, entry, exportPreview, exportRow, previewRow } from '@/test/fixtures';
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

/** Answers the confirm step with `answer`; `planned` and `exported` hold the requests. */
function backend(answer: ExportPreview | { throws: unknown }) {
  const planned: ExportRequest[] = [];
  const exported: ExportRequest[] = [];
  mockIPC((cmd, args) => {
    const req = (args as { req: ExportRequest }).req;
    if (cmd === 'plan_export') {
      planned.push(req);
      if ('throws' in answer) throw answer.throws;
      return { ...answer, rows: answer.rows.filter((r) => req.fileIds.includes(r.fileId)) };
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
  useSettings.setState({ ...initialSettings, place: 'inPlace' }, true);
  useRecovery.setState({ running: false });
  useExportFlow.setState({ phase: 'closed', request: null, preview: null, error: null, leftOut: 0, differing: [], along: [], planned: [], again: false, stays: '', offer: '' });
  // A pipeline left running by a test before this one.
  pipeline().send({ type: 'FINISHED' });
});

describe('the confirm step', () => {
  it('asks the engine first and writes nothing until confirmed', async () => {
    add(1, 2);
    const { planned, exported } = backend(exportPreview([previewRow(1, cutRow), previewRow(2, skipRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByText('2 files: 1 gain, 1 cut, 1 skipped; backups in ~/Music/SoundCheck Backups');
    expect(planned[0]?.fileIds).toEqual([1, 2]);
    expect(exported).toEqual([]);
    expect(useLibrary.getState().rows[1]?.state).toBe('analysed');

    // One file to write; the skipped one still goes along, to get its row.
    await userEvent.click(screen.getByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(exported).toHaveLength(1));
    expect(exported[0]?.fileIds).toEqual([1, 2]);
    expect(useExportFlow.getState().phase).toBe('closed');
    // What it planned is what the rows show while they wait.
    expect(useLibrary.getState().rows[1]).toMatchObject({ state: 'exportQueued', export: cutRow });
  });

  it('sends back each file as it was planned, and the settings revision', async () => {
    add(1, 2);
    const { exported } = backend(exportPreview([previewRow(1, cutRow), previewRow(2, skipRow)], { revision: 7 }));
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(exported).toHaveLength(1));
    expect(exported[0]?.confirmed).toEqual({
      revision: 7,
      files: [
        { fileId: 1, identity: 'id-1', sameName: false },
        { fileId: 2, identity: 'id-2', sameName: false },
      ],
    });
  });

  it('leaves a file it could not plan out of the job', async () => {
    add(1, 2);
    const { exported } = backend(exportPreview([previewRow(1, cutRow), previewRow(2, null)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(exported).toHaveLength(1));
    expect(exported[0]?.fileIds).toEqual([1]);
  });

  it('shows the target the numbers were planned with', async () => {
    add(1);
    backend(exportPreview([previewRow(1, cutRow)], { decide: { mode: 'dj', target: -8, ceiling: -0.5, bpmRange: [70, 180] } }));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText(/DJ target -8\.0 LUFS · ceiling -0\.5 dBTP/)).toBeInTheDocument();
  });

  it('says what is written in its title and on its button, not XML-only files', async () => {
    add(1, 2);
    const xml = exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } } });
    backend(exportPreview([previewRow(1, cutRow), previewRow(2, xml)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByRole('heading', { name: 'Write 1 file' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Write 1 file' })).toBeEnabled();
  });

  it('offers only the XML when that is all there is to do', async () => {
    add(1);
    const xml = exportRow({ stage: 'skipped', action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } } });
    backend(exportPreview([previewRow(1, xml)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByRole('button', { name: 'Update the rekordbox XML' })).toBeEnabled();
  });

  it('says plainly that a Prepare cut shifts positions, and how far', async () => {
    add(1);
    backend(exportPreview([previewRow(1, cutRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    const warning = await screen.findByText(/Positions shift/);
    expect(warning).toHaveTextContent('Positions shift on 1 cut file, by up to 0.21 s');
    expect(warning).toHaveTextContent('Library');
  });

  it('says nothing about positions when nothing is cut', async () => {
    add(1);
    backend(
      exportPreview([
        previewRow(1, exportRow({ action: { type: 'write', gainDb: -1, cut: { type: 'library' } }, positionsShift: false })),
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Write 1 file' });
    expect(screen.queryByText(/Positions shift/)).toBeNull();
  });

  it('lists each skipped file with the row\'s own words, as sentences', async () => {
    add(1, 2);
    backend(exportPreview([previewRow(1, cutRow), previewRow(2, skipRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    const list = await screen.findByRole('list', { name: 'File by file' });
    expect(within(list).getByText('Track 2')).toBeInTheDocument();
    expect(within(list).getByText(/Skipped: It has no audio above/)).toBeInTheDocument();
  });

  it('shows a file that could not be planned', async () => {
    add(1);
    backend(exportPreview([previewRow(1, null, { error: { kind: 'corrupt', message: 'truncated', fileId: 1 } })]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText(/Cannot be planned: Truncated/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Nothing to write' })).toBeDisabled();
  });

  it('says which earlier copies it replaces and which files in the folder it leaves', async () => {
    add(1, 2);
    useSettings.setState({ place: 'folder', outDir: '/Out' });
    const foreign = exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'otherFileInFolder' } }, positionsShift: false });
    backend(
      exportPreview([previewRow(1, cutRow, { replacesCopy: true }), previewRow(2, foreign)], {
        backupFolder: null,
        artefactsFolder: '/Out',
      }),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText('1 file replaces your earlier copy.')).toBeInTheDocument();
    expect(screen.getByText("1 file in the folder isn't SoundCheck's, skipped.")).toBeInTheDocument();
  });

  it('counts a copy of another track apart from an earlier copy of its own', async () => {
    add(1, 2);
    useSettings.setState({ place: 'folder', outDir: '/Out' });
    const other = exportRow({ stage: 'skipped', action: { type: 'skip', reason: { type: 'copyOfOtherTrack' } }, positionsShift: false });
    backend(
      exportPreview([previewRow(1, cutRow, { replacesCopy: true }), previewRow(2, other)], {
        backupFolder: null,
        artefactsFolder: '/Out',
      }),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText('1 file skipped: the folder has a copy of that name made from another file.')).toBeInTheDocument();
    expect(screen.getByText('1 file replaces your earlier copy.')).toBeInTheDocument();
    expect(screen.queryByText(/isn't SoundCheck's/)).toBeNull();
  });

  it('names an artefact it cannot read, and how to go on', async () => {
    add(1);
    backend({
      throws: {
        kind: 'corrupt',
        message:
          'corrupt file /Out/grid-report.csv: it starts like one SoundCheck wrote but cannot be read, so what it lists would be lost. Nothing was written: move it away or choose another folder',
        fileId: null,
      },
    });
    render(<ExportConfirm />);
    void openExportConfirm();
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('/Out/grid-report.csv');
    expect(alert).toHaveTextContent('choose another folder');
    expect(screen.queryByRole('button', { name: /^Write/ })).toBeNull();
  });

  it('a file the plan found changed asks for an analysis, and lights Analyse', async () => {
    add(1);
    backend(
      exportPreview([
        previewRow(1, null, { error: { kind: 'fileChanged', message: 'changed since it was analysed', fileId: 1 } }),
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Nothing to write' });
    expect(useLibrary.getState().rows[1]?.changedOnDisk).toBe(true);
    expect(staleIds(useLibrary.getState(), [70, 180])).toEqual([1]);
  });

  it('says the folder\'s XML keeps what it lists', async () => {
    add(1);
    useSettings.setState({ place: 'folder', outDir: '/Out' });
    backend(exportPreview([previewRow(1, cutRow)], { backupFolder: null, artefactsFolder: '/Out', xmlListed: 5 }));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText(/the 5 tracks listed there stay listed/)).toBeInTheDocument();
  });

  it('leaves out the rows not analysed, and says so', async () => {
    add(1);
    useLibrary.getState().add([entry(2)]);
    const { planned } = backend(exportPreview([previewRow(1, cutRow)]));
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
    expect(screen.queryByRole('button', { name: /^(Write|Export)/ })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(useExportFlow.getState().phase).toBe('closed');
  });

  it('Cancel and Esc write nothing, and a late answer lands nowhere', async () => {
    add(1);
    const { exported } = backend(exportPreview([previewRow(1, cutRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Write 1 file' });
    await userEvent.keyboard('{Escape}');
    expect(useExportFlow.getState().phase).toBe('closed');
    expect(exported).toEqual([]);

    const opening = openExportConfirm();
    dismissExportConfirm();
    await opening;
    expect(useExportFlow.getState()).toMatchObject({ phase: 'closed', preview: null });
  });

  it('a double click on Export does not dismiss it: the backdrop waits a moment', async () => {
    add(1);
    backend(exportPreview([previewRow(1, cutRow)]));
    const clock = vi.spyOn(performance, 'now').mockReturnValue(1000);
    const { container } = render(<ExportConfirm />);
    void openExportConfirm();
    const dialog = await screen.findByRole('dialog');
    const backdrop = dialog.previousElementSibling as HTMLElement;
    expect(container).toBeDefined();
    clock.mockReturnValue(1100);
    fireEvent.mouseDown(backdrop);
    expect(useExportFlow.getState().phase).not.toBe('closed');
    clock.mockReturnValue(1400);
    fireEvent.mouseDown(backdrop);
    expect(useExportFlow.getState().phase).toBe('closed');
    clock.mockRestore();
  });

  it('sends the folder and the mode the Serato rule picked', async () => {
    const e = entry(1);
    useLibrary.getState().add([{ ...e, info: { ...e.info, serato: true } }]);
    useLibrary.getState().applyEvent(analysed(1));
    useSettings.setState({ place: 'folder', outDir: '/Out', depth: 16, xml: false });
    const { planned } = backend(exportPreview([previewRow(1, cutRow)]));
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
    const { planned } = backend(exportPreview([]));
    useRecovery.setState({ running: true });
    await openExportConfirm();
    expect(useExportFlow.getState().phase).toBe('closed');
    expect(planned).toEqual([]);
  });

  it('closes the settings drawer of a narrow window: one modal panel at a time', async () => {
    add(1);
    backend(exportPreview([previewRow(1, cutRow)]));
    usePanels.getState().setSettings(true);
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('dialog');
    expect(usePanels.getState().settings).toBe(false);
  });

  it('closes the grid view of a file about to be replaced, and says where it went', async () => {
    add(1);
    backend(exportPreview([previewRow(1, cutRow)]));
    const close = vi.fn(async () => {});
    useTrack.setState({ fileId: 1, close });
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    expect(close).toHaveBeenCalled();
    expect(useLibrary.getState().info).toMatch(/grid view closed: Track 1 is being exported/);
    useTrack.setState({ fileId: null });
  });
});

describe('a target typed and not yet committed', () => {
  it('reaches the engine before anything is planned', async () => {
    add(1);
    const order: string[] = [];
    let landed: (value: unknown) => void = () => {};
    mockIPC((cmd) => {
      if (cmd === 'set_decide_settings') {
        order.push('settings sent');
        return new Promise((go) => {
          landed = (v) => {
            order.push('settings landed');
            go(v);
          };
        });
      }
      if (cmd === 'plan_export') {
        order.push('planned');
        return exportPreview([previewRow(1, cutRow)]);
      }
      return undefined;
    });
    useSettings.setState({ hydrated: false });
    const stop = startSettingsSync();
    await waitFor(() => expect(order).toContain('settings sent'));
    landed({ revision: 1, plans: [] });
    await waitFor(() => expect(order).toContain('settings landed'));
    order.length = 0;
    useSettings.setState({ place: 'inPlace' });

    render(
      <>
        <Rail />
        <ExportConfirm />
      </>,
    );
    const target = screen.getByRole('textbox', { name: /Target/ });
    await userEvent.clear(target);
    await userEvent.type(target, '-8');
    expect(document.activeElement).toBe(target);
    expect(useSettings.getState().target).toBe(-11);

    // A press on a button keeps the field's focus in the app: fireEvent does not move it either.
    fireEvent.click(screen.getByRole('button', { name: /^Export/ }));
    await waitFor(() => expect(order).toEqual(['settings sent']));
    expect(useSettings.getState().target).toBe(-8);
    expect(useExportFlow.getState().phase).toBe('planning');

    landed({ revision: 2, plans: [] });
    await waitFor(() => expect(order).toEqual(['settings sent', 'settings landed', 'planned']));
    stop();
  });
});

describe('rows exported with other settings', () => {
  const now = {
    decide: { mode: 'dj' as const, target: -11, ceiling: -0.5, bpmRange: [70, 180] as [number, number] },
    settings: { batchMode: 'prepare' as const, place: 'inPlace' as const, depth: null, gridOnly: false, tbpm: true, xml: true, leadMs: 5 },
    outDir: null,
  };

  /** Row 1 exported at -8, row 2 new; the settings say -11. */
  function exported() {
    add(1, 2);
    useLibrary.getState().beginExport([1], []);
    useLibrary.getState().applyEvent({
      type: 'exportDone',
      jobId: 3,
      fileId: 1,
      row: doneRow({ exportedWith: { ...now, decide: { ...now.decide, target: -8 } } }),
      inPlace: false,
      update: null,
    });
    useLibrary.getState().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
  }

  it('stay unless the unticked box is ticked', async () => {
    exported();
    const { planned, exported: sent } = backend(exportPreview([previewRow(1, cutRow), previewRow(2, cutRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByText('1 exported at -8.0 stay as they are.')).toBeInTheDocument();
    // Planned as they would go out: only the new row, until asked for the other.
    expect(planned.at(-1)?.fileIds).toEqual([2]);
    const box = screen.getByRole('checkbox', { name: 'Export them again at -11.0' });
    expect(box).not.toBeChecked();
    expect(screen.getByRole('button', { name: 'Write 1 file' })).toBeEnabled();

    await userEvent.click(box);
    expect(await screen.findByRole('button', { name: 'Write 2 files' })).toBeEnabled();
    expect(planned.at(-1)?.fileIds).toEqual([1, 2]);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Export them again at -11.0' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]?.fileIds).toEqual([2]);
  });

  it('with nothing else to do, the box is still there to tick', async () => {
    add(1);
    useLibrary.getState().beginExport([1], []);
    useLibrary.getState().applyEvent({
      type: 'exportDone',
      jobId: 3,
      fileId: 1,
      row: doneRow({ exportedWith: { ...now, decide: { ...now.decide, target: -8 } } }),
      inPlace: false,
      update: null,
    });
    useLibrary.getState().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    const { planned } = backend(exportPreview([previewRow(1, cutRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    expect(await screen.findByRole('button', { name: 'Nothing to write' })).toBeDisabled();
    expect(planned).toEqual([]);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Export them again at -11.0' }));
    expect(await screen.findByRole('button', { name: 'Write 1 file' })).toBeEnabled();
  });

  it('go along when ticked', async () => {
    exported();
    const { exported: sent } = backend(exportPreview([previewRow(1, cutRow), previewRow(2, cutRow)]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('checkbox', { name: /Export them again/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Write 2 files' }));
    await waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0]?.fileIds).toEqual([1, 2]);
  });
});

describe('a file changed after it was confirmed', () => {
  it('fails, lights Analyse, and after a new analysis exports', async () => {
    add(1);
    const events: ((e: JobEvent) => void)[] = [];
    const requests: string[] = [];
    mockIPC((cmd, args) => {
      requests.push(cmd);
      const channel = (args as { onEvent?: { onmessage: (e: JobEvent) => void } }).onEvent;
      if (channel) events.push((e) => channel.onmessage(e));
      if (cmd === 'plan_export') return exportPreview([previewRow(1, cutRow)]);
      return cmd === 'export' || cmd === 'analyze' ? 5 : undefined;
    });
    render(<ExportConfirm />);
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(events).toHaveLength(1));
    const job = events[0] as (e: JobEvent) => void;
    act(() => {
      job({
        type: 'failed',
        jobId: 5,
        fileId: 1,
        error: { kind: 'fileChanged', message: 'it changed since you confirmed', fileId: 1 },
      });
      job({ type: 'finished', jobId: 5, cancelled: false });
    });
    expect(useLibrary.getState().rows[1]?.state).toBe('failed');
    expect(staleIds(useLibrary.getState(), useSettings.getState().bpmRange)).toEqual([1]);

    // Analyse, as its button does: the row reads the file again and is no longer stale.
    analyseStale();
    await waitFor(() => expect(requests).toContain('analyze'));
    const analysis = events[1] as (e: JobEvent) => void;
    act(() => {
      analysis(analysed(1));
      analysis({ type: 'finished', jobId: 5, cancelled: false });
    });
    expect(useLibrary.getState().rows[1]?.state).toBe('analysed');
    expect(staleIds(useLibrary.getState(), useSettings.getState().bpmRange)).toEqual([]);

    // And Export works again.
    requests.length = 0;
    void openExportConfirm();
    await userEvent.click(await screen.findByRole('button', { name: 'Write 1 file' }));
    await waitFor(() => expect(requests).toContain('export'));
    act(() => (events[2] as (e: JobEvent) => void)({ type: 'finished', jobId: 5, cancelled: false }));
  });
});

describe('why Export cannot be pressed', () => {
  const work = {
    fresh: [1, 2],
    differing: [],
    retry: [],
    along: [],
    needAnalyse: 0,
    anyExported: false,
    anyRevertible: false,
    reverting: false,
  };
  const gate = {
    work,
    hasRows: true,
    busy: false,
    xmlWriting: false,
    recoveryRunning: false,
    needsFolder: false,
  };
  const none = { ...work, fresh: [] };

  it('has a reason for each case, and none when it can', () => {
    expect(exportBlock(gate)).toBeNull();
    expect(exportBlock({ ...gate, recoveryRunning: true })).toMatch(/crash interrupted/);
    expect(exportBlock({ ...gate, busy: true })).toMatch(/job is running/);
    expect(exportBlock({ ...gate, work: { ...work, reverting: true } })).toMatch(/revert is running/);
    expect(exportBlock({ ...gate, xmlWriting: true })).toMatch(/XML/);
    expect(exportBlock({ ...gate, needsFolder: true })).toBe('Choose a folder');
    // A folder is asked for only when it is all that stands in the way.
    expect(exportBlock({ ...gate, needsFolder: true, busy: true })).toMatch(/job is running/);
    expect(exportBlock({ ...gate, needsFolder: true, work: none })).toBe('Analyse the tracks first.');
    expect(exportBlock({ ...gate, needsFolder: true, work: { ...work, needAnalyse: 1 } })).toMatch(/1 track needs Analyse/);
    expect(exportBlock({ ...gate, work: { ...work, needAnalyse: 3 } })).toBe('3 tracks need Analyse for this BPM range.');
    expect(exportBlock({ ...gate, work: none })).toBe('Analyse the tracks first.');
  });

  it('skipped rows alone do not make work', () => {
    expect(exportBlock({ ...gate, work: { ...none, along: [1] } })).toBe('Nothing new to export.');
  });

  it('rows that may no longer be skipped do', () => {
    expect(exportBlock({ ...gate, work: { ...none, retry: [1] } })).toBeNull();
  });

  it('rows exported with other settings do', () => {
    expect(exportBlock({ ...gate, work: { ...none, differing: [1] } })).toBeNull();
  });

  it('offers Revert only when there is one', () => {
    const exported = { ...none, anyExported: true };
    expect(exportBlock({ ...gate, work: exported })).not.toMatch(/Revert/);
    expect(exportBlock({ ...gate, work: { ...exported, anyRevertible: true } })).toMatch(/Revert a track/);
  });
});
