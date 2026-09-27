import { useEffect, useState } from 'react';
import { trackCover } from '@/lib/ipc';
import { displayTitle, specText } from '@/lib/format';
import { useLibrary } from '@/state/library';
import { StatusPill } from '@/features/library/cells';
import { back } from './actions';
import { useTrack } from './store';
import { Btn } from './ui';

/** Back, the cover, what the track is and its file spec, and where it stands. */
export function GridHeader() {
  const opened = useTrack((s) => s.opened);
  const fileId = useTrack((s) => s.fileId);
  const row = useLibrary((s) => (fileId === null ? undefined : s.rows[fileId]));
  const cover = useCover(fileId, opened?.cover ?? null);
  const info = opened?.entry.info;
  const title = info ? displayTitle(info, opened.entry.path) : '';
  const byline = info ? [info.artist, info.album].filter(Boolean).join(' · ') : '';
  return (
    <header className="flex h-[76px] shrink-0 items-center gap-3.5 border-b border-line bg-bg-1 pl-2 pr-[18px]">
      <Btn hint="Esc" onClick={() => void back()} aria-label="Back to the track list">
        ‹ Back
      </Btn>
      <div className="flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-md border border-line bg-bg-2">
        {cover ? <img src={cover} alt="" className="size-full object-cover" /> : null}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-[3px]">
        <div className="flex min-w-0 items-center gap-2.5">
          <h1 className="truncate text-[17px] font-semibold tracking-tight text-fg-0">{title}</h1>
          {byline && <span className="truncate text-[13px] text-fg-1">{byline}</span>}
          {row && <StatusPill row={row} />}
        </div>
        {info && (
          <span className="font-mono text-[11.5px] font-medium text-fg-1">{specText(info)}</span>
        )}
      </div>
    </header>
  );
}

/** A Blob URL for the open track's embedded cover, released when the track changes. */
function useCover(fileId: number | null, mime: string | null): string | null {
  const [cover, setCover] = useState<{ fileId: number; url: string } | null>(null);
  useEffect(() => {
    if (fileId === null || mime === null) return;
    let made: string | null = null;
    let live = true;
    trackCover(fileId, mime).then(
      (blob) => {
        if (!live || blob === null) return;
        made = URL.createObjectURL(blob);
        setCover({ fileId, url: made });
      },
      () => {},
    );
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [fileId, mime]);
  return cover !== null && cover.fileId === fileId ? cover.url : null;
}
