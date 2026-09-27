import { useEffect, useRef } from 'react';
import { readoutText } from './readout';
import { useTrack } from './store';
import { useView } from './viewStore';

/** Time, bar and beat, residual and short-term loudness at the pointer (or the playhead). */
export function CursorReadout() {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const write = () => {
      const el = ref.current;
      if (!el) return;
      const text = readoutText(useTrack.getState(), useView.getState().hover);
      el.hidden = text === null;
      if (text !== null && el.textContent !== text) el.textContent = text;
    };
    write();
    const unTrack = useTrack.subscribe(write);
    const unView = useView.subscribe(write);
    return () => {
      unTrack();
      unView();
    };
  }, []);
  return (
    <div
      ref={ref}
      hidden
      aria-live="off"
      className="pointer-events-none absolute left-3 top-[34px] z-10 rounded-[5px] border border-line bg-[rgba(12,14,18,.85)] px-2 py-1 font-mono text-[11px] tabular-nums text-fg-1"
    />
  );
}
