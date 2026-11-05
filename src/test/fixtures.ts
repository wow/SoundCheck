import type {
  ExportArtefacts,
  ExportPreview,
  ExportPreviewRow,
  ExportRow,
  ExportVerification,
  FileEntry,
  JobEvent,
  Plan,
  RowAnalysis,
} from '@/lib/ipc';

/** A probed FLAC row. */
export function entry(fileId: number, title = `Track ${fileId}`): FileEntry {
  return {
    fileId,
    path: `/music/${title}.flac`,
    info: {
      codec: 'flac',
      sampleRate: 44100,
      channels: 2,
      bitsPerSample: 16,
      float: false,
      bitrateKbps: null,
      duration: 240,
      title,
      artist: 'Artist',
      album: null,
      djUnsafe: null,
      serato: false,
      seratoTags: [],
      seratoUnknown: false,
      soundcheck: null,
      soundcheckUnreadable: null,
    },
  };
}

export function analysis(overrides: Partial<RowAnalysis> = {}): RowAnalysis {
  return {
    duration: 240,
    sampleRate: 44100,
    channels: 2,
    integrated: -9.5,
    shortTermP95: -7.8,
    lra: 5,
    truePeak: -0.3,
    grid: {
      bpm: 128,
      meter: '4/4',
      fourFour: true,
      meterRunnerUp: null,
      confidence: 'green',
      reasons: [],
      verdict: 'static',
      octaveUp: null,
      octaveDown: 64,
      bar1: 0.5,
      residualP95Ms: 4,
      residualMaxMs: 9,
      driftPpm: 3,
    },
    gridSkipped: null,
    tagBpm: null,
    cached: false,
    edited: false,
    confirmed: false,
    ...overrides,
  };
}

export function plan(overrides: Partial<Plan> = {}): Plan {
  return {
    measured: -7.8,
    gain: { type: 'gain', gainDb: -3.2, shortByLu: 0, truePeakAfter: -3.5 },
    skip: null,
    review: [],
    status: 'analysed',
    ...overrides,
  };
}

export function analysed(fileId: number, p: Plan = plan(), a: RowAnalysis = analysis()): JobEvent {
  return { type: 'analysed', jobId: 1, fileId, row: a, plan: p, revision: 0 };
}

export function verification(overrides: Partial<ExportVerification> = {}): ExportVerification {
  return {
    blocks: 29,
    carried: 26,
    patched: 0,
    edited: 1,
    replaced: 2,
    dropped: 0,
    tagsAdded: 3,
    framesIn: 1000,
    framesOut: 1000,
    bitsOut: 16,
    exact: false,
    dithered: false,
    staleLoudnessTags: [],
    ...overrides,
  };
}

/** A row the engine sends for a file being written with a -2.3 dB gain and a 0.21 s cut. */
export function exportRow(overrides: Partial<ExportRow> = {}): ExportRow {
  return {
    stage: 'processing',
    action: { type: 'write', gainDb: -2.3, cut: { type: 'cut', frames: 9261, seconds: 0.21 } },
    positionsShift: true,
    gridWithheld: false,
    notices: [],
    verification: null,
    gridCheck: null,
    editCarried: false,
    revertible: false,
    exportedWith: null,
    notes: [],
    ...overrides,
  };
}

/** A row finished, verified and in place. */
export function doneRow(overrides: Partial<ExportRow> = {}): ExportRow {
  return exportRow({
    stage: 'done',
    verification: verification(),
    gridCheck: { result: 'pass', offsetMs: 0.4, bpmDiff: 0 },
    revertible: true,
    ...overrides,
  });
}

/** A row of the confirm step's plan. */
export function previewRow(
  fileId: number,
  row: ExportRow | null,
  overrides: Partial<ExportPreviewRow> = {},
): ExportPreviewRow {
  return {
    fileId,
    row,
    error: row ? null : { kind: 'corrupt', message: 'unreadable', fileId },
    identity: row ? `id-${fileId}` : null,
    replacesCopy: false,
    ...overrides,
  };
}

export function exportPreview(rows: ExportPreviewRow[], overrides: Partial<ExportPreview> = {}): ExportPreview {
  return {
    rows,
    revision: 4,
    decide: { mode: 'dj', target: -11, ceiling: -0.5, bpmRange: [70, 180] },
    xmlListed: 0,
    backupFolder: '/Users/dj/Music/SoundCheck Backups',
    artefactsFolder: '/Users/dj/Music/SoundCheck/exports',
    ...overrides,
  };
}

export function artefacts(overrides: Partial<ExportArtefacts> = {}): ExportArtefacts {
  return {
    folder: '/exports/2026-11-05',
    xml: '/exports/2026-11-05/soundcheck-rekordbox.xml',
    report: '/exports/2026-11-05/grid-report.csv',
    listed: 3,
    ...overrides,
  };
}
