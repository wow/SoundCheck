/**
 * A stand-in for the Rust side, for developing and screenshotting the UI in a plain browser
 * (`pnpm dev:mock`). It streams synthetic rows through the same IPC types, covering every Action
 * and Status the table can show. Never bundled into the app: `main.tsx` imports it only when
 * `VITE_MOCK_IPC` is set.
 */
import { mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import type {
  Codec,
  DecideSettings,
  FileEntry,
  GainPlan,
  JobEvent,
  Plan,
  ReviewReason,
  RowAnalysis,
  RowGrid,
  RowPlan,
} from '@/lib/ipc';

interface Synthetic {
  title: string;
  artist: string;
  codec: Codec;
  rate: number;
  bits: number | null;
  kbps: number | null;
  float?: boolean;
  sp95: number | null;
  integrated: number | null;
  tp: number;
  bpm: number | null;
  meter?: string;
  confidence?: RowGrid['confidence'];
  verdict?: RowGrid['verdict'];
  reasons?: RowGrid['reasons'];
  fails?: boolean;
}

const LIBRARY: Synthetic[] = [
  { title: 'Night Drive', artist: 'Mock Artist A', codec: 'flac', rate: 44100, bits: 16, kbps: null, sp95: -7.8, integrated: -9.1, tp: -0.3, bpm: 124 },
  { title: 'Aksak Groove', artist: 'Mock Artist B', codec: 'flac', rate: 44100, bits: 16, kbps: null, sp95: -8.4, integrated: -9.9, tp: -0.2, bpm: 250, meter: '9/8 · 2+2+2+3', confidence: 'amber', verdict: 'drifts', reasons: ['meterMargin', 'drifts'] },
  { title: 'Quiet Classic', artist: 'Mock Artist C', codec: 'aiff', rate: 44100, bits: 16, kbps: null, sp95: -14.6, integrated: -16.2, tp: -3.7, bpm: 116 },
  { title: 'Radio Edit', artist: 'Mock Artist D', codec: 'mp3', rate: 44100, bits: null, kbps: 320, sp95: -13.2, integrated: -14.4, tp: -0.8, bpm: 126 },
  { title: 'Float Bounce', artist: 'Mock Artist E', codec: 'wav', rate: 44100, bits: 32, kbps: null, float: true, sp95: -8.1, integrated: -9.6, tp: 0.3, bpm: 87, confidence: 'amber', verdict: 'staticWarn', reasons: ['octaveMargin'] },
  { title: 'Lossless Import', artist: 'Mock Artist F', codec: 'alac', rate: 44100, bits: 16, kbps: null, sp95: -10.2, integrated: -11.5, tp: -0.2, bpm: 124 },
  { title: 'Already There', artist: 'Mock Artist G', codec: 'flac', rate: 48000, bits: 24, kbps: null, sp95: -11.02, integrated: -12.3, tp: -0.6, bpm: 120 },
  { title: 'Six Eight Waltz', artist: 'Mock Artist H', codec: 'flac', rate: 44100, bits: 16, kbps: null, sp95: -9.5, integrated: -11.2, tp: -1.1, bpm: 138.41, meter: '6/8 · 3+3' },
  { title: 'Broken Download', artist: 'Mock Artist I', codec: 'mp3', rate: 44100, bits: null, kbps: 256, sp95: null, integrated: null, tp: 0, bpm: null, fails: true },
  { title: 'Big Room', artist: 'Mock Artist J', codec: 'wav', rate: 44100, bits: 24, kbps: null, sp95: -6.1, integrated: -7.4, tp: 0.4, bpm: 128 },
  { title: 'Downtempo', artist: 'Mock Artist K', codec: 'flac', rate: 44100, bits: 16, kbps: null, sp95: -12.2, integrated: -14.0, tp: -2.9, bpm: 92 },
  { title: 'Breakbeat Tool', artist: 'Mock Artist L', codec: 'aiff', rate: 44100, bits: 24, kbps: null, sp95: -9.0, integrated: -10.8, tp: -0.4, bpm: 174, confidence: 'red', reasons: ['octaveMargin', 'downbeatMargin'] },
];

const STEP = 1.505149978319906;

let nextId = 1;
const entries = new Map<number, Synthetic>();
const analysed = new Map<number, RowAnalysis>();
let settings: DecideSettings = { mode: 'dj', target: -11, ceiling: -0.5, bpmRange: [70, 180] };
let revision = 0;
const cancelled = new Set<number>();
let nextJob = 1;

function entryFor(s: Synthetic, folder: string): FileEntry {
  const fileId = nextId++;
  entries.set(fileId, s);
  return {
    fileId,
    path: `${folder}/${s.artist} - ${s.title}.${s.codec === 'alac' ? 'm4a' : s.codec}`,
    info: {
      codec: s.codec,
      sampleRate: s.rate,
      channels: 2,
      bitsPerSample: s.bits,
      float: s.float ?? false,
      bitrateKbps: s.kbps,
      duration: 240,
      title: s.title,
      artist: s.artist,
      album: null,
      djUnsafe: s.float ? 'float' : null,
    },
  };
}

function analysisFor(s: Synthetic): RowAnalysis {
  return {
    duration: 240,
    sampleRate: s.rate,
    channels: 2,
    integrated: s.integrated,
    shortTermP95: s.sp95,
    lra: 6,
    truePeak: s.tp,
    grid:
      s.bpm === null
        ? null
        : {
            bpm: s.bpm,
            meter: s.meter ?? '4/4',
            fourFour: !s.meter,
            meterRunnerUp: s.meter ? '4/4' : null,
            confidence: s.confidence ?? 'green',
            reasons: s.reasons ?? [],
            verdict: s.verdict ?? 'static',
            octaveUp: s.bpm * 2 <= 250 ? s.bpm * 2 : null,
            octaveDown: s.bpm / 2 >= 60 ? s.bpm / 2 : null,
            bar1: 0.52,
            residualP95Ms: s.verdict ? 24 : 5,
            residualMaxMs: s.verdict ? 31 : 11,
            driftPpm: s.verdict === 'drifts' ? 180 : 4,
          },
    gridSkipped: s.bpm === null ? 'no beats found' : null,
    tagBpm: null,
    cached: false,
  };
}

/** The same rules as the engine's decide, enough for a believable table. */
function planFor(s: Synthetic, a: RowAnalysis): Plan {
  const measured = settings.mode === 'dj' ? a.shortTermP95 : a.integrated;
  const review: ReviewReason[] = [];
  const g = a.grid;
  if (g) {
    if (g.confidence !== 'green') review.push({ type: 'confidence' });
    if (g.verdict === 'drifts') review.push({ type: 'drifts' });
    if (g.bpm < settings.bpmRange[0] || g.bpm > settings.bpmRange[1]) review.push({ type: 'outsideBpmRange' });
  }
  if (!['wav', 'aiff', 'flac', 'mp3'].includes(s.codec)) {
    return { measured, gain: null, skip: { type: 'analyseOnly', codec: s.codec }, review, status: 'skipped' };
  }
  if (measured === null) return { measured, gain: null, skip: { type: 'silent' }, review, status: 'skipped' };
  const desired = settings.target - measured;
  const headroom = Math.max(settings.ceiling - a.truePeak, 0);
  let gain: GainPlan;
  if (Math.abs(desired) < 0.05) gain = { type: 'atTarget' };
  else if (s.codec === 'mp3') {
    let steps = Math.round(desired / STEP);
    if (steps > 0) steps = Math.min(steps, Math.floor(headroom / STEP));
    gain = { type: 'globalGain', steps, gainDb: steps * STEP, residualLu: desired - steps * STEP, truePeakAfter: a.truePeak + steps * STEP };
  } else {
    const g2 = desired <= 0 ? desired : Math.min(desired, headroom);
    const short = desired - g2;
    gain = { type: 'gain', gainDb: g2, shortByLu: short < 0.05 ? 0 : short, truePeakAfter: a.truePeak + g2 };
  }
  return { measured, gain, skip: null, review, status: review.length > 0 ? 'needsReview' : 'analysed' };
}

function send(channel: unknown, event: JobEvent) {
  (channel as Channel<JobEvent>).onmessage(event);
}

function runJob(jobId: number, fileIds: number[], channel: unknown) {
  let done = 0;
  const started = Date.now();
  fileIds.forEach((fileId, i) => {
    const s = entries.get(fileId);
    const at = 300 + i * 450;
    setTimeout(() => {
      if (cancelled.has(jobId)) return;
      send(channel, { type: 'started', jobId, fileId });
      for (const f of [0.2, 0.5, 0.8]) {
        setTimeout(() => !cancelled.has(jobId) && send(channel, { type: 'progress', jobId, fileId, fraction: f }), f * 800);
      }
      setTimeout(() => {
        if (cancelled.has(jobId) || !s) return;
        if (s.fails) {
          send(channel, { type: 'failed', jobId, fileId, error: { kind: 'corrupt', message: 'The MP3 stream ends after 12 s of 240 s', fileId } });
        } else {
          const a = analysisFor(s);
          analysed.set(fileId, a);
          send(channel, { type: 'analysed', jobId, fileId, row: a, plan: planFor(s, a), revision });
        }
        done += 1;
        const secs = (Date.now() - started) / 1000;
        send(channel, { type: 'batch', jobId, done, total: fileIds.length, etaMs: Math.round(((fileIds.length - done) * 450)), realtimeX: (done * 240) / secs });
        if (done === fileIds.length) send(channel, { type: 'finished', jobId, cancelled: false });
      }, 900);
    }, at);
  });
}

export function installMockBackend(): void {
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'app_version':
        return '0.0.3-mock';
      case 'expand_paths': {
        const folder = String((a.paths as string[] | undefined)?.[0] ?? '/Music');
        return LIBRARY.map((s) => entryFor(s, folder));
      }
      case 'analyze': {
        const req = a.req as { fileIds: number[] };
        const jobId = nextJob++;
        runJob(jobId, req.fileIds, a.onEvent);
        return jobId;
      }
      case 'cancel_job': {
        cancelled.add(a.jobId as number);
        return true;
      }
      case 'set_decide_settings': {
        settings = a.settings as DecideSettings;
        revision += 1;
        const plans: RowPlan[] = [];
        for (const [fileId, an] of analysed) {
          const s = entries.get(fileId);
          if (s) plans.push({ fileId, plan: planFor(s, an) });
        }
        return { revision, plans };
      }
      case 'calibration_target':
        return -9.2;
      case 'restore_session':
        return { revision, rows: [] };
      case 'clear_session':
        analysed.clear();
        entries.clear();
        return null;
      case 'plugin:store|load':
        return 1;
      case 'plugin:store|get':
        return [null, false];
      case 'plugin:dialog|open':
        return ['/Music/Mock Crate'];
      case 'plugin:dialog|message':
        return true;
      default:
        return null;
    }
  });
}
