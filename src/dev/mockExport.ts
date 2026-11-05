/**
 * The export commands of the browser stand-in: every Action, Status and grid-check chip the table
 * can show, in the order the real job sends them. Reached through `mockBackend.ts` only.
 */
import type { Channel } from '@tauri-apps/api/core';
import type {
  Cut,
  ExportAction,
  ExportPreview,
  ExportRequest,
  ExportRow,
  ExportVerification,
  FileEntry,
  GridCheck,
  JobEvent,
  RevertResult,
  RowAnalysis,
  WriteXmlRequest,
  XmlWritten,
} from '@/lib/ipc';
import { analysisFor, planFor, type Synthetic } from './mockBackend';

export interface MockSession {
  analysed: Map<number, RowAnalysis>;
  entries: Map<number, Synthetic>;
  revision: () => number;
  entryFor: (s: Synthetic, fileId: number) => FileEntry;
}

const BACKUPS = '/Users/mock/Music/SoundCheck Backups';
const EXPORTS = '/Users/mock/Music/SoundCheck/exports';
const STEP_MS = 350;

const meter = (beatsPerBar: number, grouping: number[]) => ({ beatsPerBar, unit: 'eighth', grouping }) as const;

/** One grid check per track, so a batch shows them all. */
const CHECKS: Record<string, GridCheck> = {
  'Night Drive': { result: 'pass', offsetMs: 0.6, bpmDiff: 0.01 },
  'Quiet Classic': { result: 'offBy', offsetMs: 12.4, bpmDiff: 0 },
  'Big Room': { result: 'checkBar1', offsetMs: 468.8, bpmDiff: 0, beats: 1 },
  'Already There': {
    result: 'advisory',
    disagreement: { kind: 'offBy', offsetMs: -14.2, bpmDiff: 0 },
  },
  'Serato Cued': { result: 'bpmDiffers', bpmDiff: 0.12, found: 122.12 },
  'Float Bounce': { result: 'noGridFound' },
  Downtempo: { result: 'meterDiffers', found: meter(9, [2, 2, 2, 3]), exported: meter(10, [3, 2, 2, 3]) },
};

const WHY = {
  mp3: 'MP3 (file writes arrive later)',
  unsupported: (label: string) => `${label} files are analysed only; SoundCheck does not write them`,
  seratoCut: (s: number) =>
    `cutting ${s.toFixed(2)} s in place would move its Serato cue points, which are stored as positions in the audio`,
  gridOnlyFlac: 'grid only on FLAC (writing tags alone to FLAC arrives later)',
  withheld: 'its BPM and bar 1 are not written, because the grid needs review (confirm it in the app)',
};

interface Planned {
  action: ExportAction;
  notes: string[];
  withheld: boolean;
}

function plannedFor(s: Synthetic, req: ExportRequest): Planned {
  const { batchMode, gridOnly, place } = req.settings;
  const write = (gainDb: number, cut: Cut, notes: string[] = [], withheld = false): Planned => ({
    action: { type: 'write', gainDb, cut },
    notes,
    withheld,
  });
  if (s.codec === 'mp3') {
    return { action: { type: 'xmlOnly', reason: { type: 'mp3OrAac', codec: 'mp3' } }, notes: [WHY.mp3], withheld: false };
  }
  if (!['wav', 'aiff', 'flac'].includes(s.codec)) {
    return {
      action: { type: 'skip', reason: { type: 'unsupported', codec: s.codec } },
      notes: [WHY.unsupported(s.codec.toUpperCase())],
      withheld: false,
    };
  }
  if (gridOnly) {
    if (s.codec === 'flac') {
      return { action: { type: 'xmlOnly', reason: { type: 'gridOnlyFlac' } }, notes: [WHY.gridOnlyFlac], withheld: false };
    }
    return write(0, { type: 'gridOnly' });
  }
  const gain = -11 - (s.sp95 ?? -11);
  const headroom = Math.max(-0.5 - s.tp, 0);
  const gainDb = gain <= 0 ? gain : Math.min(gain, headroom);
  if (batchMode === 'library') return write(gainDb, { type: 'library' });
  if (s.confidence && s.confidence !== 'green') {
    const bar1 = 22_932;
    return write(gainDb, { type: 'needsReview', bar1, bar1S: 0.52 }, [WHY.withheld], true);
  }
  const seconds = 0.21;
  if (s.serato && place === 'inPlace') {
    return {
      action: { type: 'skip', reason: { type: 'seratoInPlaceCut', cutS: seconds } },
      notes: [WHY.seratoCut(seconds)],
      withheld: false,
    };
  }
  return write(gainDb, { type: 'cut', frames: Math.round(seconds * s.rate), seconds });
}

function row(planned: Planned, stage: ExportRow['stage'], place: ExportRequest['settings']['place']): ExportRow {
  const cut = planned.action.type === 'write' && planned.action.cut.type === 'cut';
  return {
    stage,
    action: planned.action,
    positionsShift: cut,
    gridWithheld: planned.withheld,
    notices: [],
    verification: null,
    gridCheck: null,
    editCarried: false,
    revertible: stage !== 'processing' && stage !== 'skipped' && place === 'inPlace',
    notes: planned.notes,
  };
}

function verification(s: Synthetic): ExportVerification {
  return {
    blocks: 29,
    carried: 26,
    patched: 0,
    edited: 1,
    replaced: 2,
    dropped: 0,
    tagsAdded: 3,
    framesIn: 240 * s.rate,
    framesOut: 240 * s.rate,
    bitsOut: s.bits ?? 16,
    exact: false,
    dithered: false,
    staleLoudnessTags: [],
  };
}

function preview(session: MockSession, req: ExportRequest): ExportPreview {
  return {
    rows: req.fileIds.map((fileId) => {
      const s = session.entries.get(fileId);
      if (!s) return { fileId, row: null, error: { kind: 'invalidArgument', message: 'not in the list', fileId } };
      const planned = plannedFor(s, req);
      const stage = planned.action.type === 'write' ? 'processing' : 'skipped';
      return { fileId, row: row(planned, stage, req.settings.place), error: null };
    }),
    backupFolder: req.settings.place === 'inPlace' ? BACKUPS : null,
    artefactsFolder: req.settings.xml ? (req.outDir ?? EXPORTS) : null,
  };
}

const cancelled = new Set<number>();
let nextJob = 1000;

function runExport(session: MockSession, jobId: number, req: ExportRequest, channel: unknown) {
  const send = (event: JobEvent) => (channel as Channel<JobEvent>).onmessage(event);
  const place = req.settings.place;
  const inPlace = place === 'inPlace';
  let done = 0;
  const total = req.fileIds.length;
  const after = (ms: number, then: () => void) =>
    setTimeout(() => {
      if (!cancelled.has(jobId)) then();
    }, ms);
  const fileDone = () => {
    done += 1;
    send({ type: 'batch', jobId, done, total, etaMs: (total - done) * STEP_MS * 4, realtimeX: 60 });
  };
  const steps = req.fileIds.map((fileId) => () => {
    const s = session.entries.get(fileId);
    if (!s) return Promise.resolve();
    return new Promise<void>((resolve) => {
      const planned = plannedFor(s, req);
      send({ type: 'started', jobId, fileId });
      after(STEP_MS, () => {
        if (planned.action.type !== 'write') {
          send({ type: 'exportSkipped', jobId, fileId, row: row(planned, 'skipped', place) });
          fileDone();
          resolve();
          return;
        }
        send({ type: 'processing', jobId, fileId, row: row(planned, 'processing', place) });
        after(STEP_MS, () => {
          if (s.title === 'Breakbeat Tool') {
            send({
              type: 'failed',
              jobId,
              fileId,
              error: { kind: 'verifyFailed', message: 'the written file does not match its plan; the original was kept', fileId },
            });
            fileDone();
            resolve();
            return;
          }
          const written = { ...row(planned, 'writing', place), verification: verification(s) };
          send({ type: 'written', jobId, fileId, row: written });
          after(STEP_MS, () => {
            const check: GridCheck =
              CHECKS[s.title] ??
              (s.meter
                ? { result: 'notChecked', reason: { type: 'oddMeter', meter: meter(6, [3, 3]) } }
                : { result: 'pass', offsetMs: 1.1, bpmDiff: 0 });
            const finished = { ...written, stage: 'done' as const, gridCheck: check, editCarried: false };
            const update = inPlace ? updateAfter(session, s, fileId) : null;
            send({ type: 'exportDone', jobId, fileId, row: finished, inPlace, update });
            fileDone();
            resolve();
          });
        });
      });
    });
  });
  void (async () => {
    for (const step of steps) {
      await step();
      if (cancelled.has(jobId)) {
        for (const fileId of req.fileIds.slice(done)) send({ type: 'cancelled', jobId, fileId });
        break;
      }
    }
    if (req.settings.xml) {
      const folder = req.outDir ?? `${EXPORTS}/2026-11-05 14.05.33`;
      send({
        type: 'artefacts',
        jobId,
        artefacts: {
          folder,
          xml: `${folder}/soundcheck-rekordbox.xml`,
          report: `${folder}/grid-report.csv`,
          listed: Math.max(0, done - 1),
        },
      });
    }
    send({ type: 'finished', jobId, cancelled: cancelled.has(jobId) });
  })();
}

/** The row after an in-place export: at the target, its bar 1 moved. */
function updateAfter(session: MockSession, s: Synthetic, fileId: number) {
  const moved: Synthetic = { ...s, sp95: -11, tp: Math.min(s.tp, -0.5) };
  const a = analysisFor(moved);
  session.analysed.set(fileId, a);
  session.entries.set(fileId, moved);
  return { fileId, row: a, plan: planFor(moved, a), revision: session.revision() };
}

const originals = new Map<number, Synthetic>();

function revert(session: MockSession, fileId: number): Promise<RevertResult> {
  const s = originals.get(fileId) ?? session.entries.get(fileId);
  return new Promise((resolve, reject) => {
    if (!s) {
      reject(new Error('not in the list'));
      return;
    }
    // About what a 6-minute track takes, shortened: long enough to see the spinner.
    setTimeout(() => {
      const a = analysisFor(s);
      session.analysed.set(fileId, a);
      session.entries.set(fileId, s);
      resolve({
        entry: session.entryFor(s, fileId),
        update: { fileId, row: a, plan: planFor(s, a), revision: session.revision() },
        backup: `${BACKUPS}/${s.title}`,
        earlierChanges: 0,
        notes: [],
      });
    }, 2500);
  });
}

function writeXml(session: MockSession, req: WriteXmlRequest): XmlWritten {
  const folder = `${EXPORTS}/2026-11-05 14.10.02`;
  const listed = req.fileIds.filter((id) => session.entries.get(id)?.confidence === undefined);
  return {
    artefacts: {
      folder,
      xml: `${folder}/soundcheck-rekordbox.xml`,
      report: `${folder}/grid-report.csv`,
      listed: listed.length,
    },
    notListed: req.fileIds
      .filter((id) => !listed.includes(id))
      .map((fileId) => ({ fileId, why: 'its grid needs review and was not confirmed' })),
  };
}

/** The answer to an export command; `undefined` for any other command. */
export function exportCommand(cmd: string, a: Record<string, unknown>, session: MockSession): unknown {
  switch (cmd) {
    case 'plan_export': {
      // Long enough to see the step read its numbers.
      const answer = preview(session, a.req as ExportRequest);
      return new Promise((resolve) => setTimeout(() => resolve(answer), 400));
    }
    case 'export': {
      const req = a.req as ExportRequest;
      for (const id of req.fileIds) {
        const s = session.entries.get(id);
        if (s && !originals.has(id)) originals.set(id, s);
      }
      const jobId = nextJob++;
      runExport(session, jobId, req, a.onEvent);
      return jobId;
    }
    case 'cancel_job':
      cancelled.add(a.jobId as number);
      return undefined;
    case 'revert_file':
      return revert(session, a.fileId as number);
    case 'write_rekordbox_xml':
      return writeXml(session, a.req as WriteXmlRequest);
    default:
      return undefined;
  }
}
