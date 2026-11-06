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
 * the file's frames. Written in place, the file keeps its path, so rekordbox still knows it, but
 * its frames are new: the index no longer matches until rekordbox analyses the file again, and
 * that analysis makes its own beat grid. Importing the track from the rekordbox XML puts the
 * exported grid back, and Analysis Lock then stops rekordbox analysing it again or changing its
 * grid. A copy in another folder is a new file to rekordbox, analysed when it is added, so it
 * needs no notice.
 */
export const FLAC_ROW_TEXT =
  "Analyse this FLAC again in rekordbox before playing it from a CDJ: rekordbox keeps an index of where each part of a FLAC it knows sits, and writing it in place moved them, so until then the track can skip. That analysis can put rekordbox's own beat grid in place of the one written here; to keep this grid, import the track from the rekordbox XML after it, then turn on Analysis Lock, which stops rekordbox analysing it again or changing its grid.";

/** The row holds a FLAC SoundCheck wrote in place, one rekordbox may already know. */
export function flacWritten(row: Row): boolean {
  const shown = row.export;
  return (
    row.entry.info.codec === "flac" &&
    shown?.verification != null &&
    shown.exportedWith?.settings.place === "inPlace"
  );
}

/** The confirm step's line when the batch writes FLAC files in place; `null` when it writes none. */
export function flacConfirmText(count: number): string | null {
  if (count === 0) return null;
  const one = count === 1;
  const files = one ? "1 FLAC file" : `${count} FLAC files`;
  return `Analyse the ${files} written in place again in rekordbox before playing ${one ? "it" : "them"} from a CDJ: rekordbox keeps an index of where each part of a FLAC it knows sits, and writing in place moves them, so until then a track can skip. That analysis can put rekordbox's own beat grid in place of the exported one; to keep the exported grid, import ${one ? "the track" : "the tracks"} from the rekordbox XML after it, then turn on Analysis Lock.`;
}

/** How many of the planned rows write a FLAC in place (`inPlace`: the export's place). */
export function flacWrites(
  planned: readonly ExportPreviewRow[],
  rows: Record<number, Row>,
  inPlace: boolean,
): number {
  if (!inPlace) return 0;
  return planned.filter(
    (p) =>
      p.row?.action.type === "write" &&
      rows[p.fileId]?.entry.info.codec === "flac",
  ).length;
}
