import { render, screen } from '@testing-library/react';
import { mockIPC } from '@tauri-apps/api/mocks';
import App from './App';

describe('App', () => {
  it('shows the three promises and the version from the Rust side', async () => {
    mockIPC((cmd) => (cmd === 'app_version' ? '0.0.1' : undefined));
    render(<App />);
    expect(screen.getByRole('heading', { name: 'Same loudness' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Fits the grid' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Nothing lost' })).toBeInTheDocument();
    expect(await screen.findByText('v0.0.1')).toBeInTheDocument();
  });
});
