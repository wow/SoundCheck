import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { mockIPC } from '@tauri-apps/api/mocks';
import type { DecideSettings } from '@/lib/ipc';
import { useSettings } from '@/state/settings';
import { Rail } from './Rail';
import { startSettingsSync } from './sync';

const initial = useSettings.getState();
beforeEach(() => useSettings.setState(initial, true));

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
});
