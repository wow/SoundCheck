import { useSelector } from '@xstate/react';
import { useShallow } from 'zustand/react/shallow';
import type { BatchMode, Place } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { pipeline } from '@/features/pipeline/actions';
import { Section, Segmented, Toggle } from '@/features/settings/controls';
import { useLibrary, type Row } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { EXPORT_DEPTHS, useSettings } from '@/state/settings';
import { pickOutDir } from './actions';
import { exportBlock, openExportConfirm, useExportFlow } from './flow';
import { batchChoice, type BatchChoice } from './mode';
import { shortPath } from './summary';
import { useExportWork, type ExportWork } from './work';

const MODES: { id: BatchMode; label: string }[] = [
  { id: 'prepare', label: 'Prepare' },
  { id: 'library', label: 'Library' },
];

const PLACES: { id: Place; label: string }[] = [
  { id: 'inPlace', label: 'In place with backup' },
  { id: 'folder', label: 'To folder' },
];

/** The depth control's ids: its option ids must be strings. */
const depthId = (depth: number | null) => (depth === null ? 'source' : String(depth));

const DEPTHS = EXPORT_DEPTHS.map((depth) => ({
  id: depthId(depth),
  label: depth === null ? 'Source' : `${depth}-bit`,
}));

function useRows(): Row[] {
  return useLibrary(
    useShallow((s) => s.order.flatMap((id) => (s.rows[id] ? [s.rows[id]] : []))),
  );
}

/** Says why the batch is Library, or that Prepare was kept, and lets the user change it. */
function SeratoNotice({ choice }: { choice: BatchChoice }) {
  const setBatchMode = useSettings((s) => s.setBatchMode);
  const n = choice.seratoTracks;
  const tracks = `${n} ${n === 1 ? 'track holds' : 'tracks hold'} Serato cue points`;
  const folders = choice.seratoFolders.length === 1 ? 'this folder' : 'these folders';
  if (choice.auto) {
    return (
      <div role="status" className="rounded-md border border-line bg-bg-2 px-2.5 py-2 text-[12px] leading-[1.45] text-fg-1">
        Library for this batch: {tracks}, which a cut would move.{' '}
        <button
          type="button"
          className="text-accent hover:underline"
          onClick={() => setBatchMode('prepare', choice.seratoFolders)}
        >
          Use Prepare for {folders}
        </button>
      </div>
    );
  }
  if (choice.kept) {
    return (
      <div role="status" className="rounded-md border border-line bg-bg-2 px-2.5 py-2 text-[12px] leading-[1.45] text-fg-1">
        Prepare kept for {folders}: {tracks}. In place they are skipped; a cut copy keeps them late by the cut.{' '}
        <button
          type="button"
          className="text-accent hover:underline"
          onClick={() => setBatchMode('library', choice.seratoFolders)}
        >
          Use Library
        </button>
      </div>
    );
  }
  return null;
}

/** The rail's export settings. */
export function ExportSection() {
  const s = useSettings();
  const rows = useRows();
  const choice = batchChoice(s, rows);
  return (
    <Section title="Export">
      <Segmented
        label="Export mode"
        value={choice.mode}
        options={MODES}
        onChange={(mode) => s.setBatchMode(mode, choice.seratoFolders)}
      />
      <SeratoNotice choice={choice} />
      <p className="text-[11.5px] leading-[1.45] text-fg-2">
        {choice.mode === 'prepare'
          ? 'For new tracks: cut at the head, at most one beat, so bar 1 starts the file.'
          : 'For tracks already cued in a DJ app: the length never changes.'}
      </p>
      <Segmented label="Where files go" value={s.place} options={PLACES} onChange={s.setPlace} fit />
      {s.place === 'folder' && (
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-fg-1" title={s.outDir ?? undefined}>
            {s.outDir ? shortPath(s.outDir) : 'No folder chosen'}
          </span>
          <button
            type="button"
            onClick={() => void pickOutDir()}
            className="inline-flex h-7 shrink-0 items-center rounded-md border border-line px-2.5 text-[12.5px] font-medium text-fg-0 hover:bg-bg-2"
          >
            Choose…
          </button>
        </div>
      )}
      <div className="flex flex-col gap-1">
        <span className="text-[11px] text-fg-2">Depth</span>
        <Segmented
          label="Depth"
          value={depthId(s.gridOnly ? null : s.depth)}
          options={DEPTHS}
          disabled={s.gridOnly}
          onChange={(id) => s.setDepth(id === 'source' ? null : Number(id))}
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <Toggle label="Grid only (no audio change)" checked={s.gridOnly} onChange={s.setGridOnly} />
        <Toggle
          label="Write TBPM"
          checked={s.tbpm[choice.mode]}
          onChange={(on) => s.setTbpm(on, choice.mode)}
        />
        <Toggle label="Write rekordbox XML" checked={s.xml} onChange={s.setXml} />
      </div>
    </Section>
  );
}

/** What stands in Export's way, and how many rows it would write. */
function useExportGate(): { block: string | null; work: ExportWork; opening: boolean; note: string } {
  const running = useSelector(pipeline(), (p) => p.value !== 'idle');
  const work = useExportWork();
  const hasRows = useLibrary((l) => l.order.length > 0);
  const xmlWriting = useLibrary((l) => l.xmlWriting);
  const recoveryRunning = useRecovery((r) => r.running);
  const opening = useExportFlow((f) => f.phase !== 'closed');
  const place = useSettings((s) => s.place);
  const outDir = useSettings((s) => s.outDir);
  const block = exportBlock({
    work,
    hasRows,
    busy: running,
    xmlWriting,
    recoveryRunning,
    needsFolder: place === 'folder' && !outDir,
  });
  const n = work.differing.length;
  const note =
    n > 0
      ? `${n} exported with other settings stay as they are unless you ask.`
      : place === 'inPlace'
        ? 'Originals go to the backup folder; Revert puts them back.'
        : 'Originals untouched; copies go to the folder.';
  return { block, work, opening, note };
}

function exportCan(work: ExportWork): boolean {
  return work.fresh.length > 0 || work.differing.length > 0;
}

/** The Export button, kept in view at the bottom of the rail, with what stands in its way. */
export function ExportAction() {
  const { block, work, opening, note } = useExportGate();
  return (
    <div className="sticky bottom-0 -mx-[18px] -mb-4 flex flex-col gap-1.5 border-t border-line bg-bg-1 px-[18px] py-3">
      <button
        type="button"
        data-export-button
        disabled={block !== null || !exportCan(work) || opening}
        onClick={() => void openExportConfirm()}
        className={cn(
          'inline-flex h-[34px] w-full items-center justify-center rounded-[7px] bg-accent text-[13px] font-semibold text-bg-0',
          'disabled:cursor-default disabled:opacity-45',
        )}
      >
        Export{work.fresh.length > 0 ? ` ${work.fresh.length}` : ''}
      </button>
      <p className="text-[11.5px] leading-[1.4] text-fg-2">{block ?? note}</p>
    </div>
  );
}

/** The same button in the top bar of a narrow window, where the rail is behind a drawer. */
export function ExportBarButton({ className }: { className: string }) {
  const { block, work, opening, note } = useExportGate();
  return (
    <button
      type="button"
      disabled={block !== null || !exportCan(work) || opening}
      title={block ?? note}
      onClick={() => void openExportConfirm()}
      className={className}
    >
      Export
    </button>
  );
}
