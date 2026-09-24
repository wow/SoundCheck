import { useEffect, useState } from 'react';
import { appVersion } from '@/lib/ipc';

const PROMISES = [
  {
    title: 'Same loudness',
    body: 'Every track lands on one DJ target, by gain only. If the ceiling would be hit, the row says how short it is.',
  },
  {
    title: 'Fits the grid',
    body: 'Beat 1 becomes the first sample with an exact BPM, so rekordbox, Serato and Traktor agree with it. 9/8 and 6/8 included.',
  },
  {
    title: 'Nothing lost',
    body: 'Every tag, cover and cue blob is carried byte for byte and verified after writing. Originals are backed up.',
  },
] as const;

/** The empty state: the only screen until analysis lands. */
export default function App() {
  const [version, setVersion] = useState<string>('');
  useEffect(() => {
    let cancelled = false;
    appVersion()
      .then((v) => {
        if (!cancelled) setVersion(v);
      })
      .catch(() => {
        if (!cancelled) setVersion('dev');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="flex h-full flex-col bg-bg-0 text-fg-0">
      <header className="flex h-13 shrink-0 items-center gap-2 border-b border-line bg-bg-1 px-4">
        <span className="inline-flex size-5 items-center justify-center rounded-md bg-accent" aria-hidden="true">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#0c0e12" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M3 12h2l2-6 3 12 3-9 2 6 2-3h4" />
          </svg>
        </span>
        <span className="text-sm font-semibold tracking-tight">SoundCheck</span>
      </header>
      <main className="flex flex-1 flex-col items-center justify-center gap-8 p-6">
        <div className="flex flex-col items-center gap-3 text-center">
          <h1 className="text-2xl font-semibold tracking-tight">Drop a folder or tracks here</h1>
          <p className="text-fg-2">WAV · AIFF · FLAC · MP3 — M4A, AAC, ALAC, Ogg and Opus are analysed only</p>
        </div>
        <ul className="flex gap-4">
          {PROMISES.map((p) => (
            <li key={p.title} className="flex w-64 flex-col gap-2 rounded-xl border border-line bg-bg-1 p-4">
              <h2 className="text-sm font-semibold">{p.title}</h2>
              <p className="text-fg-1">{p.body}</p>
            </li>
          ))}
        </ul>
        <p className="text-fg-2">Analysis first. Nothing is written until you press Process.</p>
      </main>
      <footer className="flex h-11 shrink-0 items-center border-t border-line bg-bg-1 px-4 text-xs text-fg-2">
        <span>Open source · MIT OR Apache-2.0</span>
        <span className="ml-auto font-mono" data-testid="version">
          {version ? `v${version}` : ''}
        </span>
      </footer>
    </div>
  );
}
