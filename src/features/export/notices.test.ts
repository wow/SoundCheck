import type { Row } from '@/state/library';
import { analysis, doneRow, entry, exportRow, plan, verification } from '@/test/fixtures';
import { autotagsStale, flacConfirmText, hasStaleTags, staleTags, staleTagsText } from './notices';

function row(overrides: Partial<Row>): Row {
  return {
    entry: entry(1),
    state: 'done',
    progress: 1,
    analysis: analysis(),
    plan: plan(),
    ...overrides,
  };
}

describe('loudness tags left at the old level', () => {
  it('names each kind once, as a DJ knows it, in the order of the file', () => {
    const labels = [
      'TXXX:REPLAYGAIN_ALBUM_GAIN',
      'TXXX:REPLAYGAIN_ALBUM_PEAK',
      'RVA2',
      'COMM:iTunNORM',
      'replaygain_track_gain',
      'R128_ALBUM_GAIN',
      'ITUNNORM',
    ];
    expect(staleTags(labels).names).toEqual([
      'Album ReplayGain',
      'RVA2',
      'iTunNORM',
      'Track ReplayGain',
      'Album R128 gain',
    ]);
  });

  it('reads one tag in the singular', () => {
    expect(staleTagsText(['RVA2'])).toBe(
      'RVA2 was left as it is; players that use it will play this file at the old level.',
    );
  });

  it("keeps Serato's for its own notice", () => {
    expect(staleTags(['GEOB:Serato Autotags']).autotags).toBe(true);
    expect(staleTags(['SERATO_AUTOGAIN']).autotags).toBe(true);
    expect(staleTagsText(['GEOB:Serato Autotags', 'SERATO_AUTOTAGS'])).toBeNull();
  });

  it('says there are more when the engine cut the list short', () => {
    expect(staleTagsText(['RVA2', 'TXXX:REPLAYGAIN_AL...'])).toBe(
      'RVA2 and other loudness tags were left as they are; players that use them will play this file at the old level.',
    );
  });

  it('says nothing when there are none', () => {
    expect(staleTagsText([])).toBeNull();
  });

  it('counts a finished export only, for the footer', () => {
    const stale = verification({ staleLoudnessTags: ['RVA2'] });
    expect(hasStaleTags(row({ export: doneRow({ verification: stale }) }))).toBe(true);
    expect(hasStaleTags(row({ export: doneRow() }))).toBe(false);
    expect(
      hasStaleTags(row({ export: exportRow({ stage: 'writing', verification: stale }) })),
    ).toBe(false);
  });
});

describe("Serato's auto gain", () => {
  const tagged = {
    ...entry(1),
    info: { ...entry(1).info, serato: true, seratoTags: ['autotags' as const] },
  };

  it('is out of date once a gain was written to a file that has Autotags', () => {
    expect(autotagsStale(row({ entry: tagged, export: doneRow() }))).toBe(true);
  });

  it('is not, before the file is written, without a gain, or without Autotags', () => {
    expect(autotagsStale(row({ entry: tagged, export: exportRow() }))).toBe(false);
    const same = doneRow({ action: { type: 'write', gainDb: 0, cut: { type: 'library' } } });
    expect(autotagsStale(row({ entry: tagged, export: same }))).toBe(false);
    expect(autotagsStale(row({ export: doneRow() }))).toBe(false);
  });
});

describe('the FLAC notice of the confirm step', () => {
  it('counts the files, and is not there for none', () => {
    expect(flacConfirmText(0)).toBeNull();
    expect(flacConfirmText(1)).toMatch(/^Analyse the 1 FLAC file written in place again in rekordbox before playing it/);
  });
});
