import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import { ActionCell, StatusCell, StatusPill } from '@/features/library/cells';
import { statusLabel } from '@/features/library/status';
import type { ExportRow } from '@/lib/ipc';
import { useLibrary, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { analysis, doneRow, entry, exportRow, plan, verification } from '@/test/fixtures';
import { verifiedText } from './text';

function row(overrides: Partial<Row> = {}): Row {
  return { entry: entry(1), state: 'done', progress: 1, analysis: analysis(), plan: plan(), ...overrides };
}

const action = (r: ExportRow, stacked = false, codec: Row['entry']['info']['codec'] = 'flac') => {
  const e = entry(1);
  return render(
    <ActionCell
      row={row({ export: r, entry: { ...e, info: { ...e.info, codec } } })}
      bpmRange={[70, 180]}
      stacked={stacked}
    />,
  );
};

const initial = useLibrary.getState();
beforeEach(() => {
  useLibrary.setState(initial, true);
  useRecovery.setState({ running: false });
});

describe('the Action column of an export', () => {
  it('gain and cut with the positions-shift badge', () => {
    action(exportRow());
    expect(screen.getByText('Gain −2.3 dB · Cut 0.21 s')).toBeInTheDocument();
    expect(screen.getByText('shift')).toBeInTheDocument();
  });

  it('no badge when nothing was cut', () => {
    action(exportRow({ action: { type: 'write', gainDb: -2.3, cut: { type: 'library' } }, positionsShift: false }));
    expect(screen.getByText('Gain −2.3 dB')).toBeInTheDocument();
    expect(screen.getByText('Length kept')).toBeInTheDocument();
    expect(screen.queryByText('shift')).toBeNull();
  });

  it('in a narrow column gain and cut stack, the cut keeping its badge', () => {
    action(exportRow(), true);
    expect(screen.getByText('Gain −2.3 dB')).toBeInTheDocument();
    expect(screen.getByText('Cut 0.21 s')).toBeInTheDocument();
    expect(screen.getByText('shift')).toBeInTheDocument();
  });

  it('a cut alone, stacked, still has room for what was verified', () => {
    action(
      doneRow({ action: { type: 'write', gainDb: 0, cut: { type: 'cut', frames: 1, seconds: 0.21 } } }),
      true,
    );
    expect(screen.getByText('Cut 0.21 s')).toBeInTheDocument();
    expect(screen.getByText('Verified: 27/27 carried, 3 added')).toBeInTheDocument();
  });

  it('grid only', () => {
    action(exportRow({ action: { type: 'write', gainDb: 0, cut: { type: 'gridOnly' } }, positionsShift: false }));
    expect(screen.getByText('Grid only')).toBeInTheDocument();
  });

  it('XML only, naming the format, with why from the row', () => {
    action(
      exportRow({
        stage: 'skipped',
        action: { type: 'xmlOnly', reason: { type: 'mp3OrAac', codec: 'mp3' } },
        positionsShift: false,
        notes: ['MP3 (file writes arrive later)'],
      }),
      false,
      'mp3',
    );
    expect(screen.getByText('Grid: XML only (MP3)')).toBeInTheDocument();
    expect(screen.getByText('MP3 (file writes arrive later)')).toBeInTheDocument();
  });

  it('a skip with the row\'s own why', () => {
    action(
      exportRow({
        stage: 'skipped',
        action: { type: 'skip', reason: { type: 'silent' } },
        positionsShift: false,
        notes: ['it has no audio above the -70 LUFS gate, so there is no loudness to align'],
      }),
    );
    expect(screen.getByText('Skipped')).toBeInTheDocument();
    expect(screen.getByText(/no audio above/)).toBeInTheDocument();
  });

  it('what happens at the start when nothing is cut', () => {
    const texts: [ExportRow['action'], string][] = [
      [{ type: 'write', gainDb: 0, cut: { type: 'onBar', barLine: 10, barLineS: 0, bar1: 10, bar1S: 0 } }, 'Starts on bar 1'],
      [{ type: 'write', gainDb: 0, cut: { type: 'onBar', barLine: 10, barLineS: 0, bar1: 90, bar1S: 8 } }, 'Starts on a bar line (bar 1 at 8.00 s)'],
      [{ type: 'write', gainDb: 0, cut: { type: 'notCut', bar1: 9, bar1S: 1, firstBarLine: 9, firstBarLineS: 1 } }, 'Not cut: bar 1 1.00 s in'],
      [{ type: 'write', gainDb: 0, cut: { type: 'notCut', bar1: 90, bar1S: 9, firstBarLine: 9, firstBarLineS: 1 } }, 'Not cut: first bar line 1.00 s in (bar 1 at 9.00 s)'],
      [{ type: 'write', gainDb: 0, cut: { type: 'needsReview', bar1: 9, bar1S: 1 } }, 'Not cut: grid needs review'],
      [{ type: 'write', gainDb: 0, cut: { type: 'noGrid' } }, 'Not cut: no grid'],
    ];
    for (const [a, text] of texts) {
      const { unmount } = action(exportRow({ action: a, positionsShift: false }));
      expect(screen.getByText(text)).toBeInTheDocument();
      expect(screen.getByText('No gain')).toBeInTheDocument();
      unmount();
    }
  });

  it('a failed export says why and that the file was not changed, in red', () => {
    render(
      <ActionCell
        row={row({ state: 'failed', exportError: { kind: 'noSpace', message: 'no space left', fileId: 1 } })}
        bpmRange={[70, 180]}
      />,
    );
    expect(screen.getByText('No space left')).toHaveClass('text-err');
    expect(screen.getByText('The file was not changed')).toBeInTheDocument();
  });

  it('before any export it still shows the plan', () => {
    render(<ActionCell row={row({ state: 'analysed' })} bpmRange={[70, 180]} />);
    expect(screen.getByText('Gain −3.2 dB')).toBeInTheDocument();
  });
});

describe('what was verified', () => {
  it('counts every block but those written anew', () => {
    expect(verifiedText(verification())).toBe('Verified: 27/27 carried, 3 added');
    expect(verifiedText(verification({ carried: 25, patched: 1, edited: 1 }))).toBe('Verified: 27/27 carried, 3 added');
  });

  it('never hides a block left out', () => {
    expect(verifiedText(verification({ carried: 25, dropped: 1, edited: 1 }))).toBe('Verified: 26/27 carried, 3 added, 1 dropped');
  });

  it('says so when there is nothing to carry', () => {
    expect(verifiedText(verification({ blocks: 2, carried: 0, edited: 0, tagsAdded: 0 }))).toBe('Verified: no tags to carry');
  });
});

describe('the Status column', () => {
  it('labels every stage of an export', () => {
    expect(statusLabel(row({ state: 'exportQueued' }))).toBe('Queued');
    expect(statusLabel(row({ state: 'processing', progress: 0.42 }))).toBe('Processing 42 %');
    expect(statusLabel(row({ state: 'writing' }))).toBe('Writing');
    expect(statusLabel(row({ state: 'checking' }))).toBe('Checking');
    expect(statusLabel(row({ state: 'done', export: doneRow() }))).toBe('Done, verified');
    expect(statusLabel(row({ state: 'failed' }))).toBe('Failed');
    expect(statusLabel(row({ state: 'skipped' }))).toBe('Skipped');
  });

  it('never says verified without a verification', () => {
    expect(statusLabel(row({ state: 'done', export: doneRow({ verification: null }) }))).toBe('Done');
  });

  it('shows a spinner and Reverting while a revert runs', () => {
    const { container } = render(<StatusPill row={row({ reverting: true })} />);
    expect(screen.getByText('Reverting')).toBeInTheDocument();
    expect(container.querySelector('svg')).not.toBeNull();
  });

  it('shows the chip of a finished row', () => {
    render(<StatusCell bpmRange={[70, 180]} id={1} row={row({ export: doneRow() })} selected />);
    expect(screen.getByText('Done, verified')).toBeInTheDocument();
    expect(screen.getByText('Grid checks out')).toBeInTheDocument();
  });
});

describe('Revert on a row', () => {
  const revertButton = () => screen.queryByRole('button', { name: 'Revert to the original' });

  it('only on a row with a change to put back', () => {
    const { rerender } = render(<StatusCell bpmRange={[70, 180]} id={1} row={row({ export: doneRow(), revertible: false })} selected />);
    expect(revertButton()).toBeNull();
    rerender(<StatusCell bpmRange={[70, 180]} id={1} row={row({ export: doneRow(), revertible: true })} selected />);
    expect(revertButton()).toBeEnabled();
  });

  it('is disabled for a row of the running batch, with why, so its hold is never refused', async () => {
    const calls: string[] = [];
    mockIPC((cmd) => void calls.push(cmd));
    useLibrary.setState({ locked: new Set([1]) });
    render(<StatusCell bpmRange={[70, 180]} id={1} row={row({ revertible: true })} selected />);
    expect(revertButton()).toBeDisabled();
    expect(revertButton()).toHaveAttribute('title', expect.stringContaining('running export'));
    await userEvent.click(revertButton() as HTMLElement);
    expect(calls).not.toContain('revert_file');
  });

  it('is disabled during recovery', () => {
    useRecovery.setState({ running: true });
    render(<StatusCell bpmRange={[70, 180]} id={1} row={row({ revertible: true })} selected />);
    expect(revertButton()).toBeDisabled();
  });

  it('takes a Tab stop only on the selected row', () => {
    const { rerender } = render(<StatusCell bpmRange={[70, 180]} id={1} row={row({ revertible: true })} selected={false} />);
    expect(revertButton()).toHaveAttribute('tabindex', '-1');
    rerender(<StatusCell bpmRange={[70, 180]} id={1} row={row({ revertible: true })} selected />);
    expect(revertButton()).toHaveAttribute('tabindex', '0');
  });
});

describe('a row exported fine whose grid still asks for a look', () => {
  const review = () =>
    row({
      state: 'needsReview',
      export: doneRow({ gridCheck: { result: 'notChecked', reason: { type: 'gridWithheld' } } }),
      plan: plan({ status: 'needsReview', review: [{ type: 'drifts' }] }),
    });

  it('says Done, verified, with what it asks under it', () => {
    render(<StatusCell bpmRange={[70, 180]} id={1} row={review()} selected />);
    const pill = screen.getByText('Done, verified');
    expect(pill.closest('.text-ok')).not.toBeNull();
    expect(screen.queryByText('Needs review')).toBeNull();
    expect(screen.getByText(/^Drifts/)).toBeInTheDocument();
  });

  it('a row that needs review and has no export still says so', () => {
    render(
      <StatusCell
        bpmRange={[70, 180]}
        id={1}
        row={row({ state: 'needsReview', plan: plan({ status: 'needsReview', review: [{ type: 'drifts' }] }) })}
        selected
      />,
    );
    expect(screen.getByText('Needs review')).toBeInTheDocument();
  });
});

describe('a row exported with other settings', () => {
  const now = {
    decide: { mode: 'dj' as const, target: -8, ceiling: -0.5, bpmRange: [70, 180] as [number, number] },
    settings: {
      batchMode: 'prepare' as const,
      place: 'folder' as const,
      depth: null,
      gridOnly: false,
      tbpm: true,
      xml: true,
      leadMs: 5,
    },
    outDir: '/Out',
  };
  const with11 = { ...now, decide: { ...now.decide, target: -11 } };

  it('says how, in place of what was verified', () => {
    render(
      <ActionCell
        row={row({ export: doneRow({ exportedWith: with11 }) })}
        bpmRange={[70, 180]}
        now={now}
      />,
    );
    expect(screen.getByText('Exported at -11.0')).toBeInTheDocument();
    expect(screen.queryByText(/^Verified/)).toBeNull();
  });

  it('says nothing when it matches, and shows what was verified', () => {
    render(
      <ActionCell row={row({ export: doneRow({ exportedWith: now }) })} bpmRange={[70, 180]} now={now} />,
    );
    expect(screen.queryByText(/^Exported/)).toBeNull();
    expect(screen.getByText(/^Verified/)).toBeInTheDocument();
  });
});

describe('text from the engine', () => {
  it('reads as a sentence where the row shows it', () => {
    render(
      <ActionCell
        row={row({ state: 'failed', exportError: { kind: 'verifyFailed', message: 'the written file differs', fileId: 1 } })}
        bpmRange={[70, 180]}
      />,
    );
    expect(screen.getByText('The written file differs')).toBeInTheDocument();
  });

  it('a skip too', () => {
    action(
      exportRow({
        stage: 'skipped',
        action: { type: 'skip', reason: { type: 'silent' } },
        positionsShift: false,
        notes: ['it has no audio'],
      }),
    );
    expect(screen.getByText('It has no audio')).toBeInTheDocument();
  });
});
