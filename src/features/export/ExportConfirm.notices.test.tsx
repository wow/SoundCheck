import { render, screen } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { ExportPreview, FileEntry } from '@/lib/ipc';
import { pipeline } from '@/features/pipeline/actions';
import { useLibrary } from '@/state/library';
import { resetRecovery } from '@/state/recovery';
import { useSettings } from '@/state/settings';
import { analysed, entry, exportPreview, exportRow, previewRow } from '@/test/fixtures';
import { ExportConfirm } from './ExportConfirm';
import { openExportConfirm, useExportFlow } from './flow';

const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();

function wav(fileId: number): FileEntry {
  const e = entry(fileId);
  return { ...e, path: `/music/Track ${fileId}.wav`, info: { ...e.info, codec: 'wav' } };
}

function add(...entries: FileEntry[]) {
  useLibrary.getState().add(entries);
  for (const e of entries) useLibrary.getState().applyEvent(analysed(e.fileId));
}

function backend(answer: ExportPreview) {
  mockIPC((cmd) => (cmd === 'plan_export' ? answer : undefined));
}

const xmlOnly = exportRow({
  stage: 'skipped',
  action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } },
  positionsShift: false,
  notes: ['grid only on FLAC'],
});

beforeEach(() => {
  useLibrary.setState(initialLibrary, true);
  useSettings.setState({ ...initialSettings, place: 'inPlace' }, true);
  resetRecovery();
  useExportFlow.setState({
    phase: 'closed',
    request: null,
    preview: null,
    error: null,
    leftOut: 0,
    differing: [],
    along: [],
    planned: [],
    again: false,
    stays: '',
    offer: '',
  });
  pipeline().send({ type: 'FINISHED' });
});

describe('the confirm step, about rekordbox and FLAC', () => {
  it('says once, for the batch, that written FLAC files need analysing in rekordbox', async () => {
    add(entry(1), entry(2), wav(3));
    backend(
      exportPreview([
        previewRow(1, exportRow()),
        previewRow(2, exportRow()),
        previewRow(3, exportRow()),
      ]),
    );
    render(<ExportConfirm />);
    void openExportConfirm();
    const said = await screen.findAllByText(/written in place again in rekordbox before playing them from a CDJ/);
    expect(said).toHaveLength(1);
    expect(said[0]).toHaveTextContent(/^Analyse the 2 FLAC files written in place/);
    expect(said[0]).toHaveTextContent(/import the tracks from the rekordbox XML after it, then turn on Analysis Lock\.$/);
  });

  it('says nothing about it for FLAC copies in another folder, which rekordbox analyses as new', async () => {
    useSettings.setState({ place: 'folder', outDir: '/Users/me/Exports' });
    add(entry(1), entry(2));
    backend(exportPreview([previewRow(1, exportRow()), previewRow(2, exportRow())]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Write 2 files' });
    expect(screen.queryByText(/FLAC files? written in place/)).toBeNull();
  });

  it('says nothing about it when no FLAC is written', async () => {
    add(entry(1), wav(2));
    backend(exportPreview([previewRow(1, xmlOnly), previewRow(2, exportRow())]));
    render(<ExportConfirm />);
    void openExportConfirm();
    await screen.findByRole('button', { name: 'Write 1 file' });
    expect(screen.queryByText(/FLAC files? written in place/)).toBeNull();
  });
});
