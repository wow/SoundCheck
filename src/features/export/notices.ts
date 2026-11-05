import type { ExportPreviewRow } from '@/lib/ipc';
import type { Row } from '@/state/library';

/**
 * What a DJ should know about a file once it is written, beyond what changed in it: loudness tags
 * the gain left at the old level, Serato's auto gain, and rekordbox analysing a FLAC again. All of
 * it comes from what the engine sent for the row (the verified write, its stale tags) and what it
 * read of the file when it was added (its codec and Serato data).
 */

/** How the engine marks a list item it had to cut short to stay within the row's budget. */
const CUT_SHORT = '...';

/** A tag label as the engine reports it (`TXXX:REPLAYGAIN_ALBUM_GAIN`, `RVA2`, `SERATO_AUTOGAIN`). */
function bare(label: string): string {
  const colon = label.indexOf(':');
  return (colon >= 0 && colon <= 4 ? label.slice(colon + 1) : label).toUpperCase();
}

function isAutotags(label: string): boolean {
  const name = bare(label);
  return name === 'SERATO AUTOTAGS' || name === 'SERATO_AUTOTAGS' || name === 'SERATO_AUTOGAIN';
}

/** The name a DJ knows a loudness tag by; `null` for Serato's, which has a notice of its own. */
function plainName(label: string): string | null {
  if (isAutotags(label)) return null;
  const name = bare(label);
  if (name.startsWith('REPLAYGAIN_TRACK_')) return 'Track ReplayGain';
  if (name.startsWith('REPLAYGAIN_ALBUM_')) return 'Album ReplayGain';
  if (name.startsWith('REPLAYGAIN_')) return 'ReplayGain';
  if (name.startsWith('R128_TRACK_')) return 'Track R128 gain';
  if (name.startsWith('R128_ALBUM_')) return 'Album R128 gain';
  if (name.startsWith('R128_')) return 'R128 gain';
  if (name === 'RVA2') return 'RVA2';
  if (name === 'ITUNNORM') return 'iTunNORM';
  return label;
}

function list(names: string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1] ?? ''}`;
}

export interface StaleTags {
  /** Plain names, each once, in the file's order. */
  names: string[];
  /** The engine cut the list short: there are more than named. */
  more: boolean;
  /** Serato Autotags is among them. */
  autotags: boolean;
}

/** The loudness tags a written file kept at the old level, by name. */
export function staleTags(labels: readonly string[]): StaleTags {
  const names: string[] = [];
  let more = false;
  let autotags = false;
  for (const label of labels) {
    if (label.endsWith(CUT_SHORT)) {
      more = true;
      continue;
    }
    if (isAutotags(label)) autotags = true;
    const name = plainName(label);
    if (name && !names.includes(name)) names.push(name);
  }
  return { names, more, autotags };
}

/**
 * "Album ReplayGain, RVA2 and iTunNORM were left as they are; players that use them will play this
 * file at the old level." `null` when there is none (Serato's has its own line).
 */
export function staleTagsText(labels: readonly string[]): string | null {
  const { names, more } = staleTags(labels);
  if (names.length === 0 && !more) return null;
  const named = more
    ? [...names, names.length > 0 ? 'other loudness tags' : 'some loudness tags']
    : names;
  const many = named.length > 1 || more;
  return `${list(named)} ${many ? 'were left as they are' : 'was left as it is'}; players that use ${many ? 'them' : 'it'} will play this file at the old level.`;
}

/** Words for Serato's auto gain gone stale, as Serato names its settings. */
export const AUTOTAGS_TEXT =
  "Serato's Auto Gain for this file was measured at the old level: in Serato, analyse it again with Set Auto Gain on, or turn Auto Gain off.";

/** The written file's change, when it was written: the gain the engine applied. */
function writtenGain(row: Row): number | null {
  const shown = row.export;
  if (!shown?.verification || shown.action.type !== 'write') return null;
  return shown.action.gainDb;
}

/**
 * The file holds Serato Autotags and its gain changed: Serato's auto gain now describes the old
 * level. The engine lists the Autotags among the stale tags of exactly such a file; the Serato data
 * read when the file was added says the same when that list was cut short.
 */
export function autotagsStale(row: Row): boolean {
  const gain = writtenGain(row);
  if (gain === null) return false;
  const stale = row.export?.verification?.staleLoudnessTags ?? [];
  if (staleTags(stale).autotags) return true;
  return gain !== 0 && row.entry.info.seratoTags.includes('autotags');
}

/** The stale tags line of a written row, Serato's aside. */
export function rowStaleTagsText(row: Row): string | null {
  return writtenGain(row) === null
    ? null
    : staleTagsText(row.export?.verification?.staleLoudnessTags ?? []);
}

/** Whether the row's finished export kept loudness tags at the old level, Serato's included. */
export function hasStaleTags(row: Row): boolean {
  if (row.export?.stage !== 'done') return false;
  return rowStaleTagsText(row) !== null || autotagsStale(row);
}

/**
 * rekordbox keeps a seek index of each FLAC it has analysed, mapping positions to where they sit in
 * the file's frames; a written FLAC's frames are new, so the index no longer matches until
 * rekordbox analyses the file again, and that analysis makes its own beat grid.
 */
export const FLAC_ROW_TEXT =
  "Analyse this FLAC in rekordbox before playing it from a CDJ: rekordbox keeps an index of where each part of a FLAC sits, and writing it moved them, so until then the track can skip. That analysis can put rekordbox's own beat grid in place of the one written here.";

/** The row holds a FLAC SoundCheck wrote. */
export function flacWritten(row: Row): boolean {
  return row.entry.info.codec === 'flac' && row.export?.verification != null;
}

/** The confirm step's line when the batch writes FLAC files; `null` when it writes none. */
export function flacConfirmText(count: number): string | null {
  if (count === 0) return null;
  const files = count === 1 ? '1 FLAC file' : `${count} FLAC files`;
  const them = count === 1 ? 'it' : 'them';
  return `Analyse the ${files} in rekordbox once ${count === 1 ? 'it is' : 'they are'} written, before playing ${them} from a CDJ: rekordbox keeps an index of where each part of a FLAC sits, and writing moves them, so until then a track can skip. That analysis can put rekordbox's own beat grid in place of the exported one.`;
}

/** How many of the planned rows write a FLAC. */
export function flacWrites(
  planned: readonly ExportPreviewRow[],
  rows: Record<number, Row>,
): number {
  return planned.filter(
    (p) => p.row?.action.type === 'write' && rows[p.fileId]?.entry.info.codec === 'flac',
  ).length;
}
