/**
 * The grid view's side of the mock backend (`pnpm dev:mock`): a synthetic track per row whose
 * kicks sit on the analysed grid (on the rows marked `drifts`, a tempo that changes: 30 ms early
 * at the start, 20 ms late at the end), waveform bins
 * shaped by those kicks, refits computed by arithmetic with real residuals against them, saved
 * and confirmed edits, and a player clock. Never bundled into the app.
 */
import type { Channel } from '@tauri-apps/api/core';
import type {
  FileEntry,
  Grid,
  GridEdit,
  GridFitHeader,
  Listen,
  Meter,
  Plan,
  RowAnalysis,
  RowGrid,
  RowUpdate,
  TrackEvent,
  TrackOpened,
} from '@/lib/ipc';
import { METERS, meterText } from '@/features/grid/meters';

const RATE = 44_100;
const SECONDS = 240;

export interface MockRow {
  entry: FileEntry;
  analysis: RowAnalysis;
  plan: Plan;
  revision: number;
}

interface Open {
  fileId: number;
  row: MockRow;
  analysed: Grid;
  kicks: Float64Array;
  channel: Channel<TrackEvent>;
}

let open: Open | null = null;
/**
 * Decoding as the engine does it: frames decoded so far, reported every 100 ms, then `ready`.
 * Bins past it read as not decoded, and `read_peaks` answers after a few milliseconds, so the
 * tiles race decoding as they do in the app.
 */
const decode = {
  frames: 0,
  timer: null as ReturnType<typeof setInterval> | null,
};
const DECODE_STEPS = 15;
const NOT_DECODED = -32768;
const saved = new Map<number, { edit: GridEdit; confirmed: boolean }>();
const player = {
  playing: false,
  position: 0,
  timer: null as ReturnType<typeof setInterval> | null,
  /** Where and when playing last started: the clock follows wall time, so a throttled timer in
   * a background tab reports fewer events but never a slower clock. */
  from: 0,
  startedAt: 0,
  /** The version heard; a new track starts on the processed one. */
  listen: { version: 'processed', matched: false } as Listen,
};

function clock(): number {
  return player.playing
    ? player.from + ((performance.now() - player.startedAt) / 1000) * RATE
    : player.position;
}

function meterOf(text: string | null): Meter {
  return METERS.find((m) => meterText(m) === text) ?? METERS[0]!;
}

function gridOf(g: RowGrid): Grid {
  return {
    anchor: Math.round(g.bar1 * RATE),
    bpm: g.bpm,
    meter: meterOf(g.meter),
    meterRunnerUp: g.meterRunnerUp ? meterOf(g.meterRunnerUp) : null,
    firstDownbeatIndex: 0,
    phraseLenBars: 8,
    segments: [],
    residualP95Ms: g.residualP95Ms,
    residualMaxMs: g.residualMaxMs,
    localBpmRange: 0.01,
    driftPpm: g.driftPpm,
    verdict: g.verdict,
    confidence: g.confidence,
    reasons: g.reasons,
    alternatives: { octaveUp: g.octaveUp, octaveDown: g.octaveDown, downbeatShiftBeats: [] },
  };
}

/**
 * Kick times (samples): on the analysed lines, or on drifting rows from 30 ms early at the start
 * to 20 ms late at the end, bending at the 40 % mark as a tempo change does.
 */
function kicksOf(g: Grid): Float64Array {
  const spb = (60 * RATE) / g.bpm;
  const drifts = g.verdict === 'drifts';
  const out: number[] = [];
  for (let i = 0; ; i++) {
    const t = g.anchor + i * spb;
    if (t > SECONDS * RATE) break;
    const x = t / (SECONDS * RATE);
    const off = drifts
      ? x < 0.4
        ? -0.03 + 0.025 * (x / 0.4)
        : -0.005 + 0.025 * ((x - 0.4) / 0.6)
      : 0;
    out.push(t + off * RATE + (((i * 7919) % 13) - 6) * 3);
  }
  return Float64Array.from(out);
}

function send(event: TrackEvent) {
  open?.channel.onmessage(event);
}

function playerEvent() {
  send({
    type: 'player',
    playing: player.playing,
    position: Math.round(player.position),
    underruns: 0,
    meter: null,
    listen: player.listen,
  });
}

/** Min/max bins shaped by the kicks: a sharp attack decaying over 90 ms above a quiet bed. */
function peaks(spb: number, firstBin: number, bins: number): ArrayBuffer {
  const out = new Int16Array(2 * bins);
  const total = SECONDS * RATE;
  const decoded = decode.frames;
  const kicks = open?.kicks ?? new Float64Array(0);
  // The last kick at or before the bin (-1 before the first: a quiet intro).
  let k = -1;
  for (let i = 0; i < bins; i++) {
    const from = (firstBin + i) * spb;
    const to = from + spb;
    if (from >= total) break;
    if (to > decoded && decoded < total) {
      out[2 * i] = NOT_DECODED;
      out[2 * i + 1] = NOT_DECODED;
      continue;
    }
    while (k + 1 < kicks.length && (kicks[k + 1] ?? Infinity) <= from) k++;
    const prev = k >= 0 ? kicks[k] : undefined;
    const inside = (kicks[k + 1] ?? Infinity) < to;
    const env = inside
      ? 0.92
      : prev === undefined
        ? 0.03 + 0.06 * Math.abs(Math.sin((firstBin + i) * 0.37) * Math.sin((firstBin + i) * 0.071))
        : 0.2 + 0.72 * Math.exp(-(from - prev) / RATE / 0.09) + 0.04 * Math.sin(from / 9000);
    const wobble = 0.85 + 0.15 * Math.abs(Math.sin((firstBin + i) * 12.9898));
    const v = Math.round(Math.min(1, env * wobble) * 32_000);
    out[2 * i] = -v;
    out[2 * i + 1] = v;
  }
  return out.buffer;
}

/** Beats the start fit is fitted to, as the engine does. */
const START_BEATS = 128;

/** The analysed grid refitted by least squares to the kicks of its first 128 beats. */
function startFitted(a: Grid, kicks: Float64Array): Grid {
  const spb = (60 * RATE) / a.bpm;
  const end = a.anchor + START_BEATS * spb;
  let n = 0,
    sx = 0,
    sy = 0,
    sxx = 0,
    sxy = 0;
  for (const t of kicks) {
    if (t > end) break;
    const k = Math.round((t - a.anchor) / spb);
    n++;
    sx += k;
    sy += t;
    sxx += k * k;
    sxy += k * t;
  }
  if (n < 8) return a;
  const slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
  const anchor = Math.round((sy - slope * sx) / n);
  return { ...a, bpm: (60 * RATE) / slope, anchor };
}

/** The share of `g`'s lines up to `end` (samples) holding a kick within 20 ms. */
function share(g: Grid, kicks: Float64Array, end: number): number {
  const spb = (60 * RATE) / g.bpm;
  let lines = 0,
    held = 0,
    k = 0;
  for (let line = g.anchor; line <= end; line += spb) {
    lines++;
    while (k + 1 < kicks.length && (kicks[k] ?? 0) < line - 0.02 * RATE) k++;
    if (Math.abs((kicks[k] ?? Infinity) - line) <= 0.02 * RATE) held++;
  }
  return lines === 0 ? 0 : held / lines;
}

function applied(edit: GridEdit): Grid {
  const a = edit.fit === 'start' ? startFitted(open!.analysed, open!.kicks) : open!.analysed;
  const meter = edit.meter ?? a.meter;
  let bpm = a.bpm * Math.pow(2, edit.octave);
  if (edit.bpm !== null) bpm = edit.bpm;
  else if (edit.tempoHint !== null) {
    const hint = edit.tempoHint;
    bpm = [3, 2, 1.5, 1, 2 / 3, 0.5, 1 / 3]
      .map((r) => a.bpm / r)
      .reduce((best, b) =>
        Math.abs(Math.log(b / hint)) < Math.abs(Math.log(best / hint)) ? b : best,
      );
  }
  const spb = (60 * RATE) / bpm;
  const anchor = edit.anchor ?? Math.round(a.anchor + edit.downbeatShift * spb);
  return { ...a, meter, bpm, anchor, meterRunnerUp: edit.meter ? a.meter : a.meterRunnerUp };
}

function fit(edit: GridEdit): ArrayBuffer {
  const g = applied(edit);
  const kicks = open!.kicks;
  const spb = (60 * RATE) / g.bpm;
  const first = Math.ceil(-g.anchor / spb);
  const residuals: number[] = [];
  let k = 0;
  let matched = 0;
  for (let i = first; ; i++) {
    const line = g.anchor + i * spb;
    if (line > SECONDS * RATE) break;
    while (
      k + 1 < kicks.length &&
      Math.abs((kicks[k + 1] ?? 0) - line) < Math.abs((kicks[k] ?? 0) - line)
    )
      k++;
    const r = (((kicks[k] ?? Infinity) - line) / RATE) * 1000;
    if (Math.abs(r) <= 50) {
      residuals.push(r);
      matched++;
    } else residuals.push(Number.NaN);
  }
  const abs = residuals
    .filter((r) => !Number.isNaN(r))
    .map(Math.abs)
    .sort((x, y) => x - y);
  const p95 = abs[Math.floor(abs.length * 0.95)] ?? 0;
  const max = abs[abs.length - 1] ?? 0;
  // The matched line furthest from its kick; unmatched lines (NaN) never count.
  let worstAt = -1;
  residuals.forEach((r, i) => {
    if (!Number.isNaN(r) && (worstAt < 0 || Math.abs(r) > Math.abs(residuals[worstAt] ?? 0)))
      worstAt = i;
  });
  const grid: Grid = {
    ...g,
    residualP95Ms: p95,
    residualMaxMs: max,
    verdict: p95 < 12 && max < 30 ? 'static' : p95 < 25 && max < 50 ? 'staticWarn' : 'drifts',
    alternatives: { ...g.alternatives, octaveUp: g.bpm * 2, octaveDown: g.bpm / 2 },
  };
  const header: GridFitHeader = {
    grid,
    firstLine: first,
    lines: residuals.length,
    worstLine: worstAt >= 0 ? first + worstAt : null,
    matched,
    attacks: kicks.length,
    fitChoice: null,
  };
  if (edit.fit === 'start' || open!.analysed.verdict === 'drifts') {
    const a = open!.analysed;
    const end = a.anchor + (START_BEATS * 60 * RATE) / a.bpm;
    header.fitChoice = {
      wholeShare: share(applied({ ...edit, fit: 'whole' }), kicks, end),
      startShare: share(applied({ ...edit, fit: 'start' }), kicks, end),
      windowEndS: end / RATE,
    };
  }
  const json = new TextEncoder().encode(JSON.stringify(header));
  const buf = new ArrayBuffer(4 + json.length + 4 * residuals.length);
  const view = new DataView(buf);
  view.setUint32(0, json.length, true);
  new Uint8Array(buf, 4, json.length).set(json);
  residuals.forEach((r, i) => view.setFloat32(4 + json.length + 4 * i, r, true));
  return buf;
}

function rowGridOf(g: Grid, base: RowGrid): RowGrid {
  return {
    ...base,
    bpm: g.bpm,
    meter: meterText(g.meter),
    fourFour: meterText(g.meter) === '4/4',
    bar1: g.anchor / RATE,
    octaveUp: g.bpm * 2,
    octaveDown: g.bpm / 2,
  };
}

/** Handles a grid-view command; `undefined` for any other command. */
export function gridCommand(
  cmd: string,
  a: Record<string, unknown>,
  rowOf: (fileId: number) => MockRow | undefined,
): unknown {
  switch (cmd) {
    case 'track_open': {
      const fileId = a.fileId as number;
      const row = rowOf(fileId);
      if (!row?.analysis.grid)
        throw { kind: 'invalidArgument', message: 'no grid to open', fileId };
      const analysed = gridOf(row.analysis.grid);
      player.listen = { version: 'processed', matched: false };
      open = {
        fileId,
        row,
        analysed,
        kicks: kicksOf(analysed),
        channel: a.onEvent as Channel<TrackEvent>,
      };
      const s = saved.get(fileId);
      const edit = s?.edit ?? {
        meter: null,
        bpm: null,
        tempoHint: null,
        octave: 0,
        anchor: null,
        downbeatShift: 0,
        fit: 'whole' as const,
      };
      // Loading a track stops the player at its start, as the engine does.
      if (player.timer) clearInterval(player.timer);
      Object.assign(player, { playing: false, position: 0, timer: null, from: 0, startedAt: 0 });
      if (decode.timer) clearInterval(decode.timer);
      decode.frames = 0;
      decode.timer = setInterval(() => {
        decode.frames = Math.min(SECONDS * RATE, decode.frames + (SECONDS * RATE) / DECODE_STEPS);
        if (decode.frames < SECONDS * RATE) {
          send({ type: 'decoded', frames: decode.frames });
        } else {
          if (decode.timer) clearInterval(decode.timer);
          decode.timer = null;
          send({ type: 'ready', frames: decode.frames });
        }
      }, 100);
      const gain = row.plan.gain;
      const opened: TrackOpened = {
        entry: row.entry,
        sampleRate: RATE,
        frames: SECONDS * RATE,
        gain: gain && gain.type !== 'atTarget' ? gain.gainDb : 0,
        bpmRange: [70, 180],
        analysed,
        grid: applied(edit),
        edit,
        confirmed: s?.confirmed ?? false,
        timeline: {
          hopMs: 100,
          shortTerm: Array.from(
            { length: SECONDS * 10 },
            (_, i) => -9 - 3 * Math.abs(Math.sin(i / 300)),
          ),
        },
        cover: null,
      };
      return opened;
    }
    case 'track_close':
      if (player.timer) clearInterval(player.timer);
      if (decode.timer) clearInterval(decode.timer);
      decode.timer = null;
      player.playing = false;
      open = null;
      return null;
    case 'read_peaks': {
      // Read now, answered 5-30 ms later: an answer can land after the next decode report.
      const bytes = peaks(a.samplesPerBin as number, a.firstBin as number, a.bins as number);
      const delay = 5 + (((a.firstBin as number) * 7 + (a.samplesPerBin as number)) % 26);
      return new Promise((resolve) => setTimeout(() => resolve(bytes), delay));
    }
    case 'track_cover':
      return new ArrayBuffer(0);
    case 'track_onsets':
      return Float64Array.from(open?.kicks ?? [], (k) => k / RATE).buffer;
    case 'grid_refit':
      return fit(a.edit as GridEdit);
    case 'grid_commit': {
      const fileId = a.fileId as number;
      const row = rowOf(fileId);
      if (!open || !row?.analysis.grid)
        throw { kind: 'invalidArgument', message: 'not open', fileId };
      const edit = a.edit as GridEdit;
      const confirmed = a.confirm as boolean;
      saved.set(fileId, { edit, confirmed });
      const g = applied(edit);
      const plan: Plan =
        confirmed && row.plan.status === 'needsReview'
          ? { ...row.plan, review: [], status: 'analysed' }
          : row.plan;
      const analysis: RowAnalysis = {
        ...row.analysis,
        grid: rowGridOf(g, row.analysis.grid),
        edited: JSON.stringify(g) !== JSON.stringify(open.analysed),
        confirmed,
      };
      const update: RowUpdate = { fileId, row: analysis, plan, revision: row.revision };
      return update;
    }
    case 'player_play':
      player.from = a.from !== null && a.from !== undefined ? (a.from as number) : clock();
      player.startedAt = performance.now();
      player.playing = true;
      if (player.timer) clearInterval(player.timer);
      player.timer = setInterval(() => {
        player.position = clock();
        playerEvent();
      }, 33);
      return null;
    case 'player_pause':
      if (player.timer) clearInterval(player.timer);
      player.position = clock();
      player.playing = false;
      playerEvent();
      return null;
    case 'player_seek':
      player.position = a.to as number;
      player.from = player.position;
      player.startedAt = performance.now();
      playerEvent();
      return null;
    case 'player_set_click':
      return null;
    case 'grid_player_listen':
      player.listen = a.listen as Listen;
      playerEvent();
      return null;
    case 'player_volume':
      return null;
    default:
      return undefined;
  }
}
