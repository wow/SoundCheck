import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import { useLibrary, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import type { Codec, ExportedWith } from '@/lib/ipc';
import type { SeratoTag } from '@/lib/ipc/generated/SeratoTag';
import { analysis, doneRow, entry, exportRow, plan, verification } from '@/test/fixtures';
import { ExportDetail } from './ExportDetail';

const initial = useLibrary.getState();

/** What an export was made with, in place or to a folder. */
function madeWith(place: 'inPlace' | 'folder'): ExportedWith {
  return {
    decide: { mode: 'dj', target: -11, ceiling: -0.5, bpmRange: [70, 180] },
    settings: { batchMode: 'prepare', place, depth: null, gridOnly: false, tbpm: true, xml: true, leadMs: 5 },
    outDir: place === 'folder' ? '/Users/me/Exports' : null,
  };
}

/** Track 1 as another format, with Serato data when given. */
function track(codec: Codec, seratoTags: SeratoTag[] = []) {
  const e = entry(1);
  return { ...e, path: `/music/Track 1.${codec}`, info: { ...e.info, codec, serato: seratoTags.length > 0, seratoTags } };
}

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

  it("a failure in three lines: what happened, why in the engine's words, what to do", () => {
    const message = '/music/Track 1.flac is being reverted; try again when that has finished';
    show({ state: 'failed', exportError: { kind: 'busy', message, fileId: 1 } });
    expect(screen.getByText('Failed: the file was not changed')).toBeInTheDocument();
    expect(screen.getByText('This file is being reverted.')).toBeInTheDocument();
    expect(screen.getByText('Wait for it to finish, then try again.')).toBeInTheDocument();
  });

  it("a refusal says so, with the engine's own advice", () => {
    const message =
      '/music/Track 1.flac cannot be changed in place: it is locked (Finder: Get Info > Locked). Unlock it, or write a copy to a folder';
    show({ state: 'failed', exportError: { kind: 'inPlaceRefused', message, fileId: 1 } });
    expect(screen.getByText('Refused: the file was not changed')).toBeInTheDocument();
    expect(screen.getByText(/^This file cannot be changed in place: it is locked/)).toBeInTheDocument();
    expect(screen.getByText('Unlock it, or write a copy to a folder.')).toBeInTheDocument();
  });

  it('a file that changed on disk: what to do is Analyse', () => {
    const message = '/music/Track 1.flac changed since you confirmed the export';
    show({ state: 'failed', changedOnDisk: true, exportError: { kind: 'fileChanged', message, fileId: 1 } });
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('Press Analyse to read it again.')).toBeInTheDocument();
  });

  it('names the loudness tags a gain left at the old level, in plain words', () => {
    const stale = ['TXXX:REPLAYGAIN_ALBUM_GAIN', 'TXXX:REPLAYGAIN_ALBUM_PEAK', 'RVA2', 'COMM:iTunNORM'];
    show({ entry: track('aiff'), export: doneRow({ verification: verification({ staleLoudnessTags: stale }) }) });
    expect(
      screen.getByText(
        'Album ReplayGain, RVA2 and iTunNORM were left as they are; players that use them will play this file at the old level.',
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Serato's Auto Gain/)).toBeNull();
    expect(screen.queryByText(/Analyse this FLAC/)).toBeNull();
  });

  it("says Serato's Auto Gain is out of date only when the file has Autotags and its gain changed", () => {
    const tagged = track('wav', ['markers', 'autotags']);
    const stale = verification({ staleLoudnessTags: ['GEOB:Serato Autotags'] });
    const { unmount } = show({ entry: tagged, export: doneRow({ verification: stale }) });
    expect(screen.getByText(/^Serato's Auto Gain for this file was measured at the old level/)).toBeInTheDocument();
    // Said on a line of its own, not among the other tags.
    expect(screen.queryByText(/were left as they are|was left as it is/)).toBeNull();
    unmount();
    // No gain change: the engine lists nothing, and nothing is said.
    const same = doneRow({ action: { type: 'write', gainDb: 0, cut: { type: 'library' } } });
    const { unmount: gone } = show({ entry: tagged, export: same });
    expect(screen.queryByText(/Serato's Auto Gain/)).toBeNull();
    gone();
    // No Autotags in the file: nothing either.
    show({ entry: track('wav', ['markers']), export: doneRow() });
    expect(screen.queryByText(/Serato's Auto Gain/)).toBeNull();
  });

  it('tells a FLAC written in place to be analysed again in rekordbox, once it is written', () => {
    const { unmount } = show({ export: doneRow({ exportedWith: madeWith('inPlace') }) });
    expect(
      screen.getByText(/^Analyse this FLAC again in rekordbox before playing it from a CDJ/),
    ).toHaveTextContent(
      /import the track from the rekordbox XML after it, then turn on Analysis Lock, which stops rekordbox analysing it again or changing its grid\.$/,
    );
    unmount();
    const { unmount: queued } = show({ state: 'exportQueued', export: exportRow({ exportedWith: madeWith('inPlace') }) });
    expect(screen.queryByText(/Analyse this FLAC/)).toBeNull();
    queued();
    // A copy in another folder is new to rekordbox and analysed when it is added: nothing to say.
    show({ export: doneRow({ exportedWith: madeWith('folder') }) });
    expect(screen.queryByText(/Analyse this FLAC/)).toBeNull();
  });

  it('every note the row carries', () => {
    show({ export: doneRow({ notes: ['tags not added: the file has no ID3 tag', 'a second note'] }) });
    expect(screen.getByText('Tags not added: the file has no ID3 tag')).toBeInTheDocument();
    expect(screen.getByText('A second note')).toBeInTheDocument();
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
    expect(screen.getByText(/^It changed since it was written\. Press Analyse/)).toHaveClass('text-err');
  });

  it('Revert is disabled for a file in the running batch, with why', () => {
    useLibrary.setState({ locked: new Set([1]) });
    show({ export: doneRow(), revertible: true });
    const button = screen.getByRole('button', { name: 'Revert' });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('title', expect.stringContaining('running export'));
  });

  it('says why the XML left it out, and that its grid or file changed since', () => {
    show({ export: doneRow(), xmlNotListed: 'no grid', exportStale: true });
    expect(screen.getByText('Not in the rekordbox XML: no grid')).toBeInTheDocument();
    expect(
      screen.getByText('The grid or the file changed since this export; export it again to write it.'),
    ).toBeInTheDocument();
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
