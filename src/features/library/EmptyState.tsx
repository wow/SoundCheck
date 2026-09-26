import { chooseFiles, chooseFolders } from '@/lib/platform';
import { addPaths } from '@/features/pipeline/actions';

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

/** Before any track is added: where to drop, what SoundCheck promises. */
export function EmptyState() {
  return (
    <div className="m-[22px] flex flex-1 flex-col items-center justify-center gap-7 rounded-[14px] border-[1.5px] border-dashed border-[#343b48] bg-bg-1">
      <div className="flex flex-col items-center gap-3.5 text-center">
        <h1 className="text-[22px] font-semibold tracking-tight">Drop a folder or tracks here</h1>
        <p className="text-fg-2">WAV · AIFF · FLAC · MP3 · M4A, AAC, ALAC, Ogg and Opus are analysed only</p>
        <div className="mt-1.5 flex gap-2.5">
          <button
            type="button"
            onClick={() => void chooseFolders().then(addPaths)}
            className="inline-flex h-[30px] items-center rounded-[7px] bg-accent px-3 text-[13px] font-medium text-bg-0"
          >
            Open folder
          </button>
          <button
            type="button"
            onClick={() => void chooseFiles().then(addPaths)}
            className="inline-flex h-[30px] items-center rounded-[7px] border border-[#343b48] px-3 text-[13px] font-medium text-fg-0"
          >
            Open files
          </button>
        </div>
      </div>
      <ul className="flex gap-3.5">
        {PROMISES.map((p) => (
          <li key={p.title} className="flex w-[250px] flex-col gap-2.5 rounded-[10px] border border-line bg-bg-1 p-[18px]">
            <h2 className="text-sm font-semibold">{p.title}</h2>
            <p className="text-[12.5px] leading-normal text-fg-1">{p.body}</p>
          </li>
        ))}
      </ul>
      <p className="text-xs text-fg-2">Analysis first. Nothing is written until you press Process.</p>
    </div>
  );
}
