import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import { useLibrary, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { analysis, doneRow, entry, exportRow, plan } from '@/test/fixtures';
import { ExportDetail } from './ExportDetail';

const initial = useLibrary.getState();

function show(overrides: Partial<Row>) {
  useLibrary.setState({
    rows: {
      1: { entry: entry(1), state: 'done', progress: 1, analysis: analysis(), plan: plan(), ...overrides },
    },
    order: [1],
    selected: 1,
  });
  return render(<ExportDetail />);
}

beforeEach(() => {
  useLibrary.setState(initial, true);
  useRecovery.setState({ running: false });
  mockIPC(() => undefined);
});

describe('the row details', () => {
  it('stay away from a row with nothing to tell', () => {
    const { container } = show({ state: 'analysed' });
    expect(container).toBeEmptyDOMElement();
  });

  it('a finished row: the change, what was verified, the check in words, and Revert', () => {
    show({
      export: doneRow({ gridCheck: { result: 'offBy', offsetMs: 12.3, bpmDiff: 0 } }),
      revertible: true,
    });
    expect(screen.getByText('Gain −2.3 dB · Cut 0.21 s')).toBeInTheDocument();
    expect(screen.getByText('Verified: 27/27 carried, 3 added')).toBeInTheDocument();
    expect(screen.getByText(/own analysis puts the bar line 12 ms later/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Revert' })).toBeEnabled();
  });

  it('a copy in a folder has nothing to revert', () => {
    show({ export: doneRow({ revertible: false }), revertible: false });
    expect(screen.queryByRole('button', { name: 'Revert' })).toBeNull();
  });

  it('a skip: what, then why in the row\'s own words', () => {
    show({
      state: 'skipped',
      export: exportRow({
        stage: 'skipped',
        action: { type: 'skip', reason: { type: 'silent' } },
        notes: ['it has no audio above the -70 LUFS gate, so there is no loudness to align'],
      }),
    });
    expect(screen.getByText('Skipped')).toBeInTheDocument();
    expect(screen.getByText(/no loudness to align/)).toBeInTheDocument();
  });

  it('a failure: what, why, and that the file was not changed', () => {
    show({ state: 'failed', exportError: { kind: 'busy', message: 'raw', fileId: 1 } });
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText(/Another job is working/)).toBeInTheDocument();
    expect(screen.getByText('The file was not changed.')).toBeInTheDocument();
  });

  it('every note the row carries', () => {
    show({ export: doneRow({ notes: ['tags not added: the file has no ID3 tag', 'a second note'] }) });
    expect(screen.getByText('tags not added: the file has no ID3 tag')).toBeInTheDocument();
    expect(screen.getByText('a second note')).toBeInTheDocument();
  });

  it('a revert under way, then a refused one', () => {
    const { unmount } = show({ export: doneRow(), revertible: true, reverting: true });
    expect(screen.getByRole('status')).toHaveTextContent(/Reverting/);
    expect(screen.getByRole('button', { name: 'Reverting…' })).toBeDisabled();
    unmount();
    show({
      export: doneRow(),
      revertible: true,
      revertError: { kind: 'fileChanged', message: 'it changed since it was written', fileId: 1 },
    });
    expect(screen.getByText('it changed since it was written')).toHaveClass('text-err');
  });

  it('Revert is disabled for a file in the running batch, with why', () => {
    useLibrary.setState({ locked: new Set([1]) });
    show({ export: doneRow(), revertible: true });
    const button = screen.getByRole('button', { name: 'Revert' });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('title', expect.stringContaining('running export'));
  });

  it('says why the XML left it out, and that its grid changed since', () => {
    show({ export: doneRow(), xmlNotListed: 'no grid', exportStale: true });
    expect(screen.getByText('Not in the rekordbox XML: no grid')).toBeInTheDocument();
    expect(screen.getByText(/grid changed since this export/)).toBeInTheDocument();
  });

  it('Revert runs from here too', async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return new Promise(() => {});
    });
    show({ export: doneRow(), revertible: true });
    await userEvent.click(screen.getByRole('button', { name: 'Revert' }));
    expect(calls).toContain('revert_file');
  });
});
