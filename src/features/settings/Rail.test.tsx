import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { DecideSettings } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { DEFAULT_SETTINGS, sanitize, savedPart, useSettings } from '@/state/settings';
import { analysed, entry } from '@/test/fixtures';
import { Rail } from './Rail';
import { startSettingsSync } from './sync';

const initial = useSettings.getState();
const initialLibrary = useLibrary.getState();
beforeEach(() => {
  useSettings.setState(initial, true);
  useLibrary.setState(initialLibrary, true);
  useRecovery.setState({ running: false });
});

/** A row in `folder`, analysed. */
function addRow(id: number, folder: string, serato = false) {
  const e = entry(id);
  const row = { ...e, path: `${folder}/t${id}.wav`, info: { ...e.info, serato } };
  useLibrary.getState().add([row]);
  useLibrary.getState().applyEvent(analysed(id));
}

const pressed = (name: string) => screen.getByRole('button', { name, pressed: true });
const exportButton = () => screen.getByRole('button', { name: /^Export/ });

describe('Rail', () => {
  it('switching to Streaming replans with integrated loudness at -14 LUFS', async () => {
    const sent: DecideSettings[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'set_decide_settings') {
        sent.push((args as { settings: DecideSettings }).settings);
        return [];
      }
      return undefined;
    });
    useSettings.getState().hydrate({});
    const stop = startSettingsSync();
    render(<Rail />);
    await userEvent.click(screen.getByRole('button', { name: 'Streaming' }));
    await waitFor(() => expect(sent.at(-1)).toMatchObject({ mode: 'streaming', target: -14, ceiling: -1 }));
    expect(screen.getByRole('combobox', { name: 'Preset' })).toHaveDisplayValue('Spotify · Integrated');
    stop();
  });

  it('a typed target commits on Enter', async () => {
    mockIPC(() => []);
    render(<Rail />);
    const target = screen.getByRole('textbox', { name: /Target/ });
    await userEvent.clear(target);
    await userEvent.type(target, '-9.5{Enter}');
    expect(useSettings.getState()).toMatchObject({ target: -9.5, djTarget: -9.5, preset: 'djTarget' });
  });

  describe('Export section', () => {
    it('shows the settings with their defaults', () => {
      mockIPC(() => []);
      render(<Rail />);
      expect(pressed('Prepare')).toBeInTheDocument();
      expect(pressed('In place with backup')).toBeInTheDocument();
      expect(pressed('Source')).toBeInTheDocument();
      expect(screen.getByRole('checkbox', { name: 'Grid only (no audio change)' })).not.toBeChecked();
      expect(screen.getByRole('checkbox', { name: 'Write TBPM' })).toBeChecked();
      expect(screen.getByRole('checkbox', { name: 'Write rekordbox XML' })).toBeChecked();
    });

    it('each control lands in the saved settings', async () => {
      mockIPC(() => []);
      render(<Rail />);
      await userEvent.click(screen.getByRole('button', { name: 'Library' }));
      await userEvent.click(screen.getByRole('button', { name: 'To folder' }));
      await userEvent.click(screen.getByRole('button', { name: '24-bit' }));
      await userEvent.click(screen.getByRole('checkbox', { name: 'Write rekordbox XML' }));
      // TBPM is per mode: off by default in Library, a track's tag being the DJ app's.
      const tbpm = screen.getByRole('checkbox', { name: 'Write TBPM' });
      expect(tbpm).not.toBeChecked();
      await userEvent.click(tbpm);
      const saved = savedPart(useSettings.getState());
      expect(saved).toMatchObject({
        batchMode: 'library',
        place: 'folder',
        depth: 24,
        xml: false,
        tbpm: { prepare: true, library: true },
      });
      // What is saved reads back as it was.
      expect(sanitize(JSON.parse(JSON.stringify(saved)) as Record<string, unknown>)).toMatchObject({
        batchMode: 'library',
        place: 'folder',
        depth: 24,
        xml: false,
      });
    });

    it('saves an export setting without replanning every row', async () => {
      const calls: string[] = [];
      mockIPC((cmd) => {
        calls.push(cmd);
        return cmd === 'set_decide_settings' ? { revision: 1, plans: [] } : undefined;
      });
      useSettings.setState({ hydrated: false });
      const stop = startSettingsSync();
      await waitFor(() => expect(useSettings.getState().hydrated).toBe(true));
      await waitFor(() => expect(calls).toContain('set_decide_settings'));
      calls.length = 0;
      useSettings.getState().setMode('streaming');
      await waitFor(() => expect(calls.filter((c) => c === 'set_decide_settings')).toHaveLength(1));
      const saves = () => calls.filter((c) => c === 'plugin:store|set').length;
      const before = saves();
      useSettings.getState().setXml(false);
      await waitFor(() => expect(saves()).toBeGreaterThan(before));
      // Only the first change, which decides differently.
      expect(calls.filter((c) => c === 'set_decide_settings')).toHaveLength(1);
      stop();
    });

    it('asks for a folder for To folder, and shows the one chosen', async () => {
      mockIPC((cmd) => (cmd === 'plugin:dialog|open' ? '/Users/dj/Music/Out' : undefined));
      render(<Rail />);
      await userEvent.click(screen.getByRole('button', { name: 'To folder' }));
      expect(screen.getByText('No folder chosen')).toBeInTheDocument();
      await userEvent.click(screen.getByRole('button', { name: 'Choose…' }));
      await waitFor(() => expect(useSettings.getState().outDir).toBe('/Users/dj/Music/Out'));
      expect(screen.getByText('~/Music/Out')).toBeInTheDocument();
    });

    it('a cancelled folder picker keeps the folder set', async () => {
      mockIPC((cmd) => (cmd === 'plugin:dialog|open' ? null : undefined));
      useSettings.setState({ place: 'folder', outDir: '/kept' });
      render(<Rail />);
      await userEvent.click(screen.getByRole('button', { name: 'Choose…' }));
      expect(useSettings.getState().outDir).toBe('/kept');
    });

    it('grid only takes the depth out of it', async () => {
      mockIPC(() => []);
      useSettings.setState({ depth: 24 });
      render(<Rail />);
      expect(pressed('24-bit')).toBeEnabled();
      await userEvent.click(screen.getByRole('checkbox', { name: 'Grid only (no audio change)' }));
      expect(pressed('Source')).toBeDisabled();
    });
  });

  describe('Serato notice', () => {
    it('a Serato row in Prepare switches the batch to Library and says why', () => {
      mockIPC(() => []);
      addRow(1, '/Music/Cued', true);
      render(<Rail />);
      expect(pressed('Library')).toBeInTheDocument();
      expect(screen.getByRole('status')).toHaveTextContent('1 track holds Serato cue points, which a cut would move');
      // The saved mode is not changed: new tracks are Prepare again.
      expect(useSettings.getState().batchMode).toBe('prepare');
    });

    it('no notice without Serato data, or when Library was chosen', () => {
      mockIPC(() => []);
      addRow(1, '/Music/New');
      const { unmount } = render(<Rail />);
      expect(pressed('Prepare')).toBeInTheDocument();
      expect(screen.queryByRole('status')).toBeNull();
      unmount();
      useSettings.setState({ batchMode: 'library' });
      addRow(2, '/Music/Cued', true);
      render(<Rail />);
      expect(screen.queryByRole('status')).toBeNull();
    });

    it('choosing Prepare is remembered for the folder, and only for it', async () => {
      mockIPC(() => []);
      addRow(1, '/Music/Cued', true);
      render(<Rail />);
      await userEvent.click(screen.getByRole('button', { name: 'Use Prepare for this folder' }));
      expect(pressed('Prepare')).toBeInTheDocument();
      expect(screen.getByRole('status')).toHaveTextContent('Prepare kept for this folder');
      expect(useSettings.getState().seratoPrepareFolders).toEqual(['/Music/Cued']);
      expect(savedPart(useSettings.getState()).seratoPrepareFolders).toEqual(['/Music/Cued']);
      addRow(2, '/Music/Other', true);
      await waitFor(() => expect(pressed('Library')).toBeInTheDocument());
    });

    it('the remembered folder is read back at the next start', () => {
      mockIPC(() => []);
      useSettings.getState().hydrate(sanitize({ seratoPrepareFolders: ['/Music/Cued'] }));
      addRow(1, '/Music/Cued', true);
      render(<Rail />);
      expect(pressed('Prepare')).toBeInTheDocument();
    });

    it('Use Library takes the folder back', async () => {
      mockIPC(() => []);
      useSettings.setState({ seratoPrepareFolders: ['/Music/Cued'] });
      addRow(1, '/Music/Cued', true);
      render(<Rail />);
      await userEvent.click(screen.getByRole('button', { name: 'Use Library' }));
      expect(pressed('Library')).toBeInTheDocument();
      expect(useSettings.getState().seratoPrepareFolders).toEqual([]);
    });
  });

  describe('the Export button', () => {
    it('counts the rows it exports', () => {
      mockIPC(() => []);
      addRow(1, '/a');
      addRow(2, '/a');
      render(<Rail />);
      expect(exportButton()).toHaveTextContent('Export 2');
      expect(exportButton()).toBeEnabled();
    });

    it('is disabled with the reason while recovery runs', () => {
      mockIPC(() => []);
      addRow(1, '/a');
      useRecovery.setState({ running: true });
      render(<Rail />);
      expect(exportButton()).toBeDisabled();
      expect(screen.getByText(/crash interrupted/)).toBeInTheDocument();
    });

    it('waits for a folder when one is needed', () => {
      mockIPC(() => []);
      addRow(1, '/a');
      useSettings.setState({ place: 'folder', outDir: null });
      render(<Rail />);
      expect(exportButton()).toBeDisabled();
      expect(screen.getByText('Choose a folder first.')).toBeInTheDocument();
    });

    it('with nothing analysed there is nothing to press', () => {
      mockIPC(() => []);
      useLibrary.getState().add([entry(1)]);
      render(<Rail />);
      expect(exportButton()).toBeDisabled();
      expect(DEFAULT_SETTINGS.place).toBe('inPlace');
    });
  });
});
