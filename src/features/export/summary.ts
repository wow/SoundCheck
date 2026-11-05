import type { Cut, ExportPreview, ExportRow } from '@/lib/ipc';
import { NEGLIGIBLE_DB } from './text';

function cutSeconds(cut: Cut): number | null {
  return cut.type === 'cut' ? cut.seconds : null;
}

export interface ExportCounts {
  files: number;
  /** Files whose level changes; a file can also be cut. */
  gain: number;
  cut: number;
  /** Written with neither: grid only, or at the target already. */
  gridOnly: number;
  tagsOnly: number;
  xmlOnly: number;
  skipped: number;
  /** Files that could not be planned; the job would fail them. */
  unreadable: number;
  /** The longest cut, seconds. */
  longestCutS: number;
  /** Files the job would write or the XML would carry. */
  toDo: number;
}

/** What the confirm step's numbers are made of: what the plan says will happen to each file. */
export function countPreview(preview: ExportPreview): ExportCounts {
  const c: ExportCounts = {
    files: preview.rows.length,
    gain: 0,
    cut: 0,
    gridOnly: 0,
    tagsOnly: 0,
    xmlOnly: 0,
    skipped: 0,
    unreadable: 0,
    longestCutS: 0,
    toDo: 0,
  };
  for (const { row } of preview.rows) {
    if (!row) {
      c.unreadable += 1;
      continue;
    }
    switch (row.action.type) {
      case 'write': {
        const { gainDb, cut } = row.action;
        const gained = Math.abs(gainDb) >= NEGLIGIBLE_DB;
        const seconds = cutSeconds(cut);
        if (gained) c.gain += 1;
        if (seconds !== null) {
          c.cut += 1;
          c.longestCutS = Math.max(c.longestCutS, seconds);
        }
        if (!gained && seconds === null) {
          if (cut.type === 'gridOnly') c.gridOnly += 1;
          else c.tagsOnly += 1;
        }
        c.toDo += 1;
        break;
      }
      case 'xmlOnly':
        c.xmlOnly += 1;
        c.toDo += 1;
        break;
      case 'skip':
        c.skipped += 1;
        break;
    }
  }
  return c;
}

/** `~/Music/x` for a path under the user's home folder. */
export function shortPath(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, '~');
}

/** `12 files: 9 gain, 3 cut, 2 skipped; backups in ~/Music/SoundCheck Backups`. */
export function summaryLine(preview: ExportPreview, outDir: string | null): string {
  const c = countPreview(preview);
  const parts = [
    c.gain > 0 && `${c.gain} gain`,
    c.cut > 0 && `${c.cut} cut`,
    c.gridOnly > 0 && `${c.gridOnly} grid only`,
    c.tagsOnly > 0 && `${c.tagsOnly} tags only`,
    c.xmlOnly > 0 && `${c.xmlOnly} XML only`,
    c.skipped > 0 && `${c.skipped} skipped`,
    c.unreadable > 0 && `${c.unreadable} unreadable`,
  ].filter(Boolean);
  const files = `${c.files} ${c.files === 1 ? 'file' : 'files'}`;
  const where = preview.backupFolder
    ? `backups in ${shortPath(preview.backupFolder)}`
    : outDir
      ? `copies in ${shortPath(outDir)}`
      : null;
  return `${files}: ${parts.join(', ') || 'nothing to do'}${where ? `; ${where}` : ''}`;
}

/** A row's first note: why it is skipped, or what the plan says the user should know. */
export function firstNote(row: ExportRow): string | null {
  return row.notes[0] ?? null;
}
