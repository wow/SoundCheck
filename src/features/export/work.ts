import { useMemo } from 'react';
import type { BatchMode, DecideSettings, ExportSettings, ExportSkip, ExportedWith } from '@/lib/ipc';
import { level } from '@/lib/format';
import { useLibrary, type Row } from '@/state/library';
import {
  decideSettings,
  exportSettings,
  useSettings,
  type SavedSettings,
} from '@/state/settings';
import { batchChoice } from './mode';
import { shortPath } from './summary';

/** What an export would be made with now. */
export interface ExportNow {
  decide: DecideSettings;
  settings: ExportSettings;
  outDir: string | null;
}

export function exportNow(s: SavedSettings, mode: BatchMode): ExportNow {
  const settings = exportSettings(s, mode);
  return {
    decide: decideSettings(s),
    settings,
    outDir: settings.place === 'folder' ? s.outDir : null,
  };
}

/** {@link exportNow} for the settings and rows as they are, changing only when it does. */
export function useExportNow(): ExportNow {
  const s = useSettings();
  const mode = useLibrary((l) =>
    batchChoice(s, l.order.flatMap((id) => (l.rows[id] ? [l.rows[id]] : []))).mode,
  );
  const now = exportNow(s, mode);
  const key = JSON.stringify(now);
  return useMemo(() => JSON.parse(key) as ExportNow, [key]);
}

function depthText(depth: number | null): string {
  return depth === null ? 'source depth' : `${depth}-bit`;
}

/** How a row's export differs from one made now. */
export interface ExportDifference {
  /** What differs, the difference most likely to matter first. */
  kind: 'target' | 'place' | 'other';
  /** In the row's words: "Exported at -11.0". */
  text: string;
}

/**
 * How a row's export differs from one made now; `null` when it does not. A file only the XML
 * carries has no gain or depth to differ, only a place to be listed in. A file skipped because only
 * the XML could carry it differs when the XML does.
 */
export function exportDifference(row: Row, now: ExportNow): ExportDifference | null {
  const was = row.export?.exportedWith;
  if (!was || row.export?.stage === 'processing' || row.export?.stage === 'writing') return null;
  const action = row.export?.action;
  if (action?.type === 'skip' && action.reason.type === 'nothingToWrite' && was.settings.xml !== now.settings.xml) {
    const text = was.settings.xml ? 'Exported with the rekordbox XML' : 'Exported without the rekordbox XML';
    return differenceOf(was, now, false) ?? { kind: 'other', text };
  }
  return differenceOf(was, now, action?.type === 'xmlOnly');
}

function differenceOf(was: ExportedWith, now: ExportNow, xmlOnly: boolean): ExportDifference | null {
  const a = was.settings;
  const b = now.settings;
  const other = (text: string): ExportDifference => ({ kind: 'other', text });
  if (a.place !== b.place || was.outDir !== now.outDir) {
    const text = a.place === 'inPlace' ? 'Exported in place' : `Exported to ${shortPath(was.outDir ?? '')}`;
    return { kind: 'place', text };
  }
  if (a.batchMode !== b.batchMode) {
    return other(`Exported as ${a.batchMode === 'prepare' ? 'Prepare' : 'Library'}`);
  }
  if (xmlOnly) return null;
  if (a.gridOnly !== b.gridOnly) return other(a.gridOnly ? 'Exported grid only' : 'Exported with audio');
  if (!a.gridOnly) {
    if (was.decide.target !== now.decide.target) {
      return { kind: 'target', text: `Exported at ${level(was.decide.target)}` };
    }
    if (was.decide.ceiling !== now.decide.ceiling) {
      return other(`Exported with ceiling ${level(was.decide.ceiling)}`);
    }
    if (was.decide.mode !== now.decide.mode) {
      return other(`Exported for ${was.decide.mode === 'dj' ? 'DJ' : 'streaming'}`);
    }
    if (a.depth !== b.depth) return other(`Exported at ${depthText(a.depth)}`);
  }
  if (a.tbpm !== b.tbpm) return other(a.tbpm ? 'Exported with TBPM' : 'Exported without TBPM');
  return null;
}

/** What the confirm step says about rows that stay: how many and how they were exported, and its offer. */
export function againCopy(rows: Row[], now: ExportNow): { stays: string; offer: string } {
  const found = rows.flatMap((row) => {
    const d = exportDifference(row, now);
    return d ? [d] : [];
  });
  const same = found.length > 0 && found.every((d) => d.text === found[0]?.text);
  const how = same && found[0] ? found[0].text.replace(/^Exported/, 'exported') : 'exported with other settings';
  const offer = found.every((d) => d.kind === 'target')
    ? `Export them again at ${level(now.decide.target)}`
    : found.every((d) => d.kind === 'place') && now.outDir
      ? `Export them again to ${shortPath(now.outDir)}`
      : 'Export them again with these settings';
  return { stays: `${found.length} ${how} stay as they are`, offer };
}

/**
 * What could make a skipped file writable, as `ExportSkip::cure` says in the engine. `file`: nothing
 * here, it is about the file itself. `grid`: a saved or confirmed grid. `settings`: other export
 * settings. `disk`: what is in the folder or goes along in the batch, which can change with no word
 * to the row.
 */
export type SkipCure = 'file' | 'grid' | 'settings' | 'disk';

export function skipCure(reason: ExportSkip): SkipCure {
  switch (reason.type) {
    case 'notDjSafeRate':
    case 'unsupportedChannels':
    case 'unsupported':
    case 'silent':
      return 'file';
    case 'noGrid':
    case 'gridNeedsReview':
    case 'xmlNoGrid':
    case 'xmlGridNeedsReview':
    case 'xmlNotOptedIn':
      return 'grid';
    case 'seratoInPlaceCut':
    case 'seratoUnknownInPlaceCut':
    case 'nothingToWrite':
      return 'settings';
    case 'otherFileInFolder':
    case 'copyOfOtherTrack':
    case 'sameNameInBatch':
      return 'disk';
    default: {
      const unhandled: never = reason;
      return unhandled;
    }
  }
}

/** What the Export button has to work on. */
export interface ExportWork {
  /** Never exported, failed, or with a new grid: new work. */
  fresh: number[];
  /** Exported with other settings: they stay as they are unless the confirm step is ticked. */
  differing: number[];
  /** Skipped for something that may be cured: planned again, and work only if the plan differs. */
  retry: number[];
  /** Skipped or XML-only as before: they go along with a batch, but make none. */
  along: number[];
  /** Fresh or differing rows analysed under another BPM range, which the job would see otherwise. */
  needAnalyse: number;
  /** Some row has a finished export. */
  anyExported: boolean;
  /** Some row has a change Revert can put back. */
  anyRevertible: boolean;
  /** Some row is being reverted. */
  reverting: boolean;
}

function sameRange(a: [number, number] | undefined, b: [number, number]): boolean {
  return a === undefined || (a[0] === b[0] && a[1] === b[1]);
}

/** Which of `work`'s lists a row with an analysis belongs on, `null` when an export leaves it out. */
function workOf(row: Row, different: boolean, work: ExportWork): number[] | null {
  const made = row.export;
  if (made?.stage === 'done') {
    if (row.exportStale) return work.fresh;
    return different ? work.differing : null;
  }
  if (made?.stage !== 'skipped') return work.fresh;
  if (made.action.type === 'xmlOnly') return row.exportStale || different ? work.fresh : work.along;
  if (made.action.type !== 'skip') return work.fresh;
  switch (skipCure(made.action.reason)) {
    case 'file':
      return work.along;
    case 'grid':
      return row.exportStale || different ? work.retry : work.along;
    case 'settings':
      return different ? work.retry : work.along;
    case 'disk':
      return work.retry;
  }
}

/** Sorts the rows by what an export made now would mean for each. */
export function exportWork(rows: Row[], ids: number[], now: ExportNow): ExportWork {
  const work: ExportWork = {
    fresh: [],
    differing: [],
    retry: [],
    along: [],
    needAnalyse: 0,
    anyExported: false,
    anyRevertible: false,
    reverting: false,
  };
  const range = now.decide.bpmRange;
  rows.forEach((row, i) => {
    const id = ids[i];
    if (id === undefined) return;
    if (row.export?.stage === 'done') work.anyExported = true;
    if (row.revertible) work.anyRevertible = true;
    if (row.reverting) work.reverting = true;
    if (row.analysis === undefined) return;
    const target = workOf(row, exportDifference(row, now) !== null, work);
    if (!target) return;
    target.push(id);
    if (target !== work.along && !sameRange(row.analysedRange, range)) {
      work.needAnalyse += 1;
    }
  });
  return work;
}

/** {@link exportWork} for the rows and settings as they are, changing only when it does. */
export function useExportWork(): ExportWork {
  const now = useExportNow();
  const key = useLibrary((l) => {
    const ids = l.order.filter((id) => l.rows[id]);
    return JSON.stringify(exportWork(ids.map((id) => l.rows[id] as Row), ids, now));
  });
  return useMemo(() => JSON.parse(key) as ExportWork, [key]);
}
