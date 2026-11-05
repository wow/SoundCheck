import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import { useLibrary } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { analysed, artefacts, doneRow, entry } from '@/test/fixtures';
import { Footer } from './Footer';

const initial = useLibrary.getState();
const lib = () => useLibrary.getState();

beforeEach(() => {
  useLibrary.setState(initial, true);
  useRecovery.setState({ running: false });
  mockIPC(() => undefined);
});

/** One track exported in place. */
function finished() {
  lib().add([entry(1)]);
  lib().applyEvent(analysed(1));
  lib().beginExport([1], []);
  lib().applyEvent({ type: 'exportDone', jobId: 3, fileId: 1, row: doneRow(), inPlace: false, update: null });
}

describe('the footer', () => {
  it('shows progress and Cancel while an export runs', async () => {
    const cancelled: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'cancel_job') cancelled.push(args);
      return undefined;
    });
    lib().add([entry(1), entry(2)]);
    lib().beginExport([1, 2], []);
    lib().applyEvent({ type: 'batch', jobId: 3, done: 1, total: 2, etaMs: 65_000, realtimeX: 12 });
    render(<Footer cancelling={false} idle={false} />);
    expect(screen.getByText(/Exporting 1 \/ 2 · ETA 1:05 · 12× real time/)).toBeInTheDocument();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '1');
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  });

  it('says it is cancelling', () => {
    lib().add([entry(1)]);
    lib().beginExport([1], []);
    render(<Footer cancelling idle={false} />);
    expect(screen.getByRole('button', { name: 'Cancelling…' })).toBeDisabled();
  });

  it('links the batch\'s files after it, each showing in the Finder', async () => {
    const shown: unknown[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'plugin:opener|reveal_item_in_dir') shown.push((args as { path?: unknown; items?: unknown }));
      return undefined;
    });
    finished();
    lib().applyEvent({ type: 'artefacts', jobId: 3, artefacts: artefacts() });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    render(<Footer cancelling={false} />);
    expect(screen.getByText(/Last export: 1 exported/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show soundcheck-rekordbox.xml in the Finder' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show grid-report.csv in the Finder' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Show in Finder' }));
    await waitFor(() => expect(shown).toHaveLength(1));
    expect(JSON.stringify(shown[0])).toContain('/exports/2026-11-05');
  });

  it('says so plainly when the XML lists nothing, even though the file is there', () => {
    finished();
    lib().applyEvent({ type: 'artefacts', jobId: 3, artefacts: artefacts({ listed: 0 }) });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    render(<Footer cancelling={false} />);
    expect(screen.getByText(/The rekordbox XML lists no tracks/)).toBeInTheDocument();
    expect(screen.queryByText(/lists 0/)).toBeNull();
  });

  it('counts the tracks the XML lists, and those the XML button left out', () => {
    lib().add([entry(1), entry(2)]);
    lib().applyXml({ artefacts: artefacts({ listed: 1 }), notListed: [{ fileId: 2, why: 'no grid' }] });
    render(<Footer cancelling={false} />);
    expect(screen.getByText(/rekordbox XML lists 1 track, 1 left out/)).toBeInTheDocument();
  });

  it('shows when the artefacts could not be written, with no links', () => {
    finished();
    lib().applyEvent({
      type: 'artefactsFailed',
      jobId: 3,
      error: { kind: 'io', message: 'disk full', fileId: null },
    });
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    render(<Footer cancelling={false} />);
    expect(screen.getByText(/rekordbox XML not written/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Show in Finder' })).toBeNull();
  });

  it('with the XML off there is no artefact, no link, and no complaint', () => {
    finished();
    lib().applyEvent({ type: 'finished', jobId: 3, cancelled: false });
    render(<Footer cancelling={false} />);
    expect(screen.getByText(/Last export: 1 exported/)).toBeInTheDocument();
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByText(/XML not written/)).toBeNull();
  });

  it('tells a window reloaded during a job what it waits for', () => {
    render(<Footer cancelling={false} windingDown idle={false} />);
    expect(screen.getByText(/window reloaded during a job/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'rekordbox XML' })).toBeDisabled();
  });

  describe('the rekordbox XML button', () => {
    const button = () => screen.getByRole('button', { name: 'rekordbox XML' });

    it('works for analysed rows with no export', async () => {
      const asked: unknown[] = [];
      mockIPC((cmd, args) => {
        if (cmd === 'write_rekordbox_xml') {
          asked.push(args);
          return { artefacts: artefacts({ listed: 1 }), notListed: [] };
        }
        return undefined;
      });
      lib().add([entry(1)]);
      lib().applyEvent(analysed(1));
      render(<Footer cancelling={false} />);
      await userEvent.click(button());
      await waitFor(() => expect(asked).toHaveLength(1));
      expect(await screen.findByText(/rekordbox XML lists 1 track/)).toBeInTheDocument();
    });

    it('rests with no row to list, while a job runs and during recovery', () => {
      const { rerender } = render(<Footer cancelling={false} />);
      expect(button()).toBeDisabled();
      lib().add([entry(1)]);
      lib().applyEvent(analysed(1));
      rerender(<Footer cancelling={false} />);
      expect(button()).toBeEnabled();
      rerender(<Footer cancelling={false} idle={false} />);
      expect(button()).toBeDisabled();
      rerender(<Footer cancelling={false} />);
      useRecovery.setState({ running: true });
      rerender(<Footer cancelling={false} />);
      expect(button()).toBeDisabled();
    });
  });
});
