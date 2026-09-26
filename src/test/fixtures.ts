import type { FileEntry, JobEvent, Plan, RowAnalysis } from '@/lib/ipc';

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
