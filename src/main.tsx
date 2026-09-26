import React from 'react';
import ReactDOM from 'react-dom/client';
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/jetbrains-mono/400.css';
import '@fontsource/jetbrains-mono/500.css';
import '@fontsource/jetbrains-mono/600.css';
import './index.css';
import App from './App';

async function start() {
  // Browser harness only: `pnpm dev:mock` replaces the Rust side with synthetic data.
  if (import.meta.env.VITE_MOCK_IPC === '1') {
    const { installMockBackend } = await import('./dev/mockBackend');
    installMockBackend();
  }
  const root = document.getElementById('root');
  if (!root) throw new Error('missing #root');
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
  if (import.meta.env.VITE_MOCK_IPC === '1' && new URLSearchParams(location.search).has('demo')) {
    const { addPaths } = await import('./features/pipeline/actions');
    void addPaths(['/Music/Mock Crate']);
  }
}

void start();
