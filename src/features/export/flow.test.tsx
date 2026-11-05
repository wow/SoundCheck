import { act, render, screen, waitFor } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ExportPreview, ExportPreviewRow, ExportRequest, ExportRow } from '@/lib/ipc';
import { pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { analysed, doneRow, entry, exportPreview, exportRow, previewRow } from '@/test/fixtures';
import { ExportConfirm } from './ExportConfirm';
import { exportNow } from './work';
import {
  confirmExport,
  dismissExportConfirm,
  openExportConfirm,
  setExportAgain,
  useExportFlow,
} from './flow';

const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();

/** Opens the step: not awaited, since opening ends only when its plan answers. */
async function open() {
  await act(async () => {
    void openExportConfirm();
  });
}

/** Plans still held, answered after each test so that none waits on them. */
let outstanding: (() => void)[] = [];

/** A backend whose plans answer only when told, in the order they were asked. */
function heldPlans() {
  const plans: {
    req: ExportRequest;
    answer: (p: ExportPreview) => void;
    fail: (e: unknown) => void;
  }[] = [];
  const exported: ExportRequest[] = [];
  let running = 0;
  let most = 0;
  mockIPC((cmd, args) => {
    const req = (args as { req: ExportRequest }).req;
    if (cmd === 'plan_export') {
      running += 1;
      most = Math.max(most, running);
      return new Promise((go, no) => {
        outstanding.push(() => no(new Error('the test is over')));
        plans.push({
          req,
          answer: (p) => {
            running -= 1;
            go(p);
          },
          fail: (e) => {
            running -= 1;
            no(e);
          },
        });
      });
    }
    if (cmd === 'export') {
      exported.push(req);
      return 5;
    }
    return undefined;
  });
  return { plans, exported, most: () => most };
}

function rowsOf(req: ExportRequest): ExportPreviewRow[] {
  return req.fileIds.map((fileId) => previewRow(fileId, exportRow(), { identity: `id-${fileId}` }));
}

function writeButton(): HTMLButtonElement {
  return screen
    .getAllByRole('button')
    .find((b) =>
      /^Write|^Nothing to write|^Export$/.test(b.textContent ?? ''),
    ) as HTMLButtonElement;
}

/** Row 1 is new work; row 2 was exported at another target, so it stays unless ticked. */
function setUpDiffering() {
  useLibrary.getState().add([entry(1), entry(2)]);
  useLibrary.getState().applyEvent(analysed(1));
  useLibrary.getState().applyEvent(analysed(2));
  const now = exportNow(useSettings.getState(), 'prepare');
  const was: ExportRow = doneRow({ exportedWith: { ...now, decide: { ...now.decide, target: now.decide.target - 3 } } });
  useLibrary.setState((l) => ({ rows: { ...l.rows, 2: { ...l.rows[2]!, export: was } } }));
}

beforeEach(() => {
  useLibrary.setState(initialLibrary, true);
  useSettings.setState({ ...initialSettings, place: 'inPlace' }, true);
  useRecovery.setState({ running: false });
  dismissExportConfirm();
  pipeline().send({ type: 'FINISHED' });
});

afterEach(async () => {
  dismissExportConfirm();
  await act(async () => {
    for (const end of outstanding) end();
    outstanding = [];
  });
});

describe('one plan at a time', () => {
  it('does not start a plan while an older one still holds the files, and Write waits for both', async () => {
    const backend = heldPlans();
    useLibrary.getState().add([entry(1)]);
    useLibrary.getState().applyEvent(analysed(1));
    render(<ExportConfirm />);

    await open();
    await waitFor(() => expect(backend.plans).toHaveLength(1));

    // Cancel and press Export again while the engine is still on the first plan.
    act(() => dismissExportConfirm());
    await open();
    expect(backend.plans).toHaveLength(1);
    expect(useExportFlow.getState().phase).toBe('planning');
    expect(writeButton()).toBeDisabled();

    // The superseded plan answers: it lands nowhere, and only now the next one starts.
    await act(async () => {
      backend.plans[0]?.answer(exportPreview(rowsOf(backend.plans[0].req)));
    });
    await waitFor(() => expect(backend.plans).toHaveLength(2));
    expect(useExportFlow.getState().phase).toBe('planning');
    expect(useExportFlow.getState().preview).toBeNull();
    expect(writeButton()).toBeDisabled();

    await act(async () => {
      backend.plans[1]?.answer(exportPreview(rowsOf(backend.plans[1].req)));
    });
    await waitFor(() => expect(useExportFlow.getState().phase).toBe('ready'));
    expect(writeButton()).toBeEnabled();
    expect(backend.most()).toBe(1);
  });

  it('keeps Write disabled after a tick and an untick until the older plan has answered', async () => {
    const backend = heldPlans();
    setUpDiffering();
    render(<ExportConfirm />);
    await open();
    await waitFor(() => expect(backend.plans).toHaveLength(1));
    await act(async () => {
      backend.plans[0]?.answer(exportPreview(rowsOf(backend.plans[0].req)));
    });
    await waitFor(() => expect(useExportFlow.getState().phase).toBe('ready'));

    // Tick: the long plan starts. Untick while it runs: the short one has to wait for it.
    await act(async () => setExportAgain(true));
    await waitFor(() => expect(backend.plans).toHaveLength(2));
    await act(async () => setExportAgain(false));
    expect(backend.plans).toHaveLength(2);
    expect(useExportFlow.getState().phase).toBe('planning');
    act(() => confirmExport());
    expect(backend.exported).toHaveLength(0);

    await act(async () => {
      backend.plans[1]?.answer(exportPreview(rowsOf(backend.plans[1].req)));
    });
    await waitFor(() => expect(backend.plans).toHaveLength(3));
    expect(useExportFlow.getState().phase).toBe('planning');
    expect(backend.plans[2]?.req.fileIds).toEqual([1]);

    await act(async () => {
      backend.plans[2]?.answer(exportPreview(rowsOf(backend.plans[2].req)));
    });
    await waitFor(() => expect(useExportFlow.getState().phase).toBe('ready'));
    expect(useExportFlow.getState().preview?.rows.map((r) => r.fileId)).toEqual([1]);
    expect(backend.most()).toBe(1);
  });

  it('skips a plan superseded while it waited, and goes on after one that failed', async () => {
    const backend = heldPlans();
    setUpDiffering();
    render(<ExportConfirm />);
    await open();
    await waitFor(() => expect(backend.plans).toHaveLength(1));

    await act(async () => setExportAgain(true));
    await act(async () => setExportAgain(false));
    await act(async () => setExportAgain(true));
    expect(backend.plans).toHaveLength(1);

    await act(async () => {
      backend.plans[0]?.fail({ kind: 'io', message: 'read failed', fileId: null });
    });
    // Only the last wish is asked, once.
    await waitFor(() => expect(backend.plans).toHaveLength(2));
    expect(backend.plans[1]?.req.fileIds).toEqual([1, 2]);
    await act(async () => {
      backend.plans[1]?.answer(exportPreview(rowsOf(backend.plans[1].req)));
    });
    await waitFor(() => expect(useExportFlow.getState().phase).toBe('ready'));
    expect(backend.plans).toHaveLength(2);
  });
});

describe('what the job is told', () => {
  it("carries the plan's same-name decision, row by row, and leaves out what it could not plan", async () => {
    const backend = heldPlans();
    useLibrary.getState().add([entry(1), entry(2), entry(3), entry(4)]);
    for (const id of [1, 2, 3, 4]) useLibrary.getState().applyEvent(analysed(id));
    render(<ExportConfirm />);
    await open();
    await waitFor(() => expect(backend.plans).toHaveLength(1));

    const taken = exportRow({
      stage: 'skipped',
      action: { type: 'skip', reason: { type: 'sameNameInBatch' } },
      positionsShift: false,
      notes: [
        'a file earlier in this export takes this name in the folder; rename one, or choose another folder',
      ],
    });
    const foreign = exportRow({
      stage: 'skipped',
      action: { type: 'skip', reason: { type: 'copyOfOtherTrack' } },
      positionsShift: false,
      notes: ['the folder has a copy'],
    });
    await act(async () => {
      backend.plans[0]?.answer(
        exportPreview([
            previewRow(1, exportRow(), { identity: 'id-1' }),
            previewRow(2, taken, { identity: 'id-2' }),
            previewRow(3, foreign, { identity: 'id-3' }),
            {
              fileId: 4,
              row: null,
              error: { kind: 'inPlaceRefused', message: 'it is a link', fileId: 4 },
              identity: null,
              replacesCopy: false,
            },
          ]),
      );
    });
    await waitFor(() => expect(useExportFlow.getState().phase).toBe('ready'));
    expect(screen.getByText(/1 refused/)).toBeInTheDocument();
    expect(screen.getByText(/Refused: /)).toBeInTheDocument();
    expect(
      screen.getByText('1 file skipped: a file earlier in this export takes its name.'),
    ).toBeInTheDocument();
    expect(writeButton()).toHaveTextContent('Write 1 file');

    act(() => confirmExport());
    await waitFor(() => expect(backend.exported).toHaveLength(1));
    expect(backend.exported[0]?.fileIds).toEqual([1, 2, 3]);
    expect(backend.exported[0]?.confirmed?.files).toEqual([
      { fileId: 1, identity: 'id-1', sameName: false },
      { fileId: 2, identity: 'id-2', sameName: true },
      { fileId: 3, identity: 'id-3', sameName: false },
    ]);
  });
});
