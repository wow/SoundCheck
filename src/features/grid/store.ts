import { create } from 'zustand';
import {
  type Grid,
  type GridEdit,
  type GridFit,
  type Listen,
  type MeterFrame,
  type TrackEvent,
  type TrackOpened,
  gridCommit,
  gridPlayerListen,
  gridRefit,
  playerPause,
  playerPlay,
  playerSeek,
  playerSetClick,
  readPeaks,
  trackClose,
  trackOnsets,
  trackOpen,
} from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { useSettings } from '@/state/settings';
import { NO_OVERS, type Overs, latched } from './meter/ballistics';
import { NO_EDIT, type History, history, isEmpty, push, redo, undo } from './edit';

/**
 * The grid view's state: the open track, the edit history, the grid shown and its fit, decoding
 * progress and the player. Every edit is refitted at once (the engine answers in a few
 * milliseconds; an older answer arriving late is dropped) and saved, unconfirmed, half a second
 * after the last change; `confirm` saves it confirmed. A nudge or a typed tempo moves the lines
 * before the answer arrives, since those need no solving.
 *
 * The engine holds one open track, and its commands run concurrently. So every command that
 * opens, closes, saves to or plays the open track goes through one queue, in the order issued:
 * a save queued before a close lands before it, and a track opened twice in a row ends with the
 * second. Each open or close starts a new session; answers and events of an older one are
 * dropped (a late save still updates the table's row).
 */

/** Wait after the last edit before it is saved unconfirmed. */
export const SAVE_DELAY_MS = 500;

export interface PlayerView {
  playing: boolean;
  /** The frame being heard. */
  position: number;
  underruns: number;
  error: string | null;
}

export interface TrackState {
  fileId: number | null;
  opened: TrackOpened | null;
  phase: 'closed' | 'opening' | 'open' | 'failed';
  /** Analysis progress while the file is analysed again (older cache entry), else null. */
  analysing: number | null;
  /** Frames decoded so far, and whether decoding has ended. */
  decoded: number;
  decodeDone: boolean;
  /** Why opening or decoding failed. */
  error: string | null;
  edits: History;
  /** The grid shown: the latest fit, or the local estimate before it arrives. */
  grid: Grid | null;
  fit: GridFit | null;
  /** Why the grid shown is not the present edit's: the engine refused it or found no grid. */
  refitNote: string | null;
  confirmed: boolean;
  /** A save is pending or running. */
  saving: boolean;
  /** Why the last save failed; cleared by the next one that succeeds. */
  saveError: string | null;
  click: boolean;
  player: PlayerView;
  /** The version heard: the user's choice at once, then what the player reports. */
  listen: Listen;
  /** The meter reading for the audio being heard; null while stopped and just after a seek. */
  meter: MeterFrame | null;
  /** IN went over 0 dBTP, OUT over the ceiling, since the track opened or the over was cleared. */
  overs: Overs;
  /** The attacks bar 1 snaps to, in seconds (empty until loaded). */
  onsets: Float64Array;
  open(fileId: number): Promise<void>;
  /** Waveform bins of the open track (for the tile cache). */
  readBins(samplesPerBin: number, firstBin: number, bins: number): Promise<Int16Array>;
  close(): Promise<void>;
  /**
   * Applies `next` as the present edit, a new undo step unless `replace` (a tap run replaces its
   * own tempo); `estimate` moves the lines at once. Ignored until the track is open.
   */
  edit(next: GridEdit, estimate?: Grid, replace?: boolean): void;
  undo(): void;
  redo(): void;
  /** Saves the present edit confirmed; resolves to whether it was saved. */
  confirm(): Promise<boolean>;
  /** Saves the present edit again after a failed save. */
  retrySave(): void;
  setClick(on: boolean): void;
  play(from?: number): void;
  pause(): void;
  seek(sample: number): void;
  /** Plays the original or the processed version, level-matched or not. */
  setListen(listen: Listen): void;
  clearOver(side: keyof Overs): void;
  /** For tests and the event channel. */
  handle(event: TrackEvent): void;
}

const IDLE_PLAYER: PlayerView = { playing: false, position: 0, underruns: 0, error: null };
/** What a newly opened track plays, as the engine starts it. */
export const OPEN_LISTEN: Listen = { version: 'processed', matched: false };

function sameListen(a: Listen, b: Listen): boolean {
  return a.version === b.version && a.matched === b.matched;
}

/** The engine's open-track commands, one after another. */
let queue: Promise<unknown> = Promise.resolve();
function enqueue<T>(job: () => Promise<T>): Promise<T> {
  const run = queue.then(job);
  queue = run.catch(() => {});
  return run;
}

/** Bumped by every open and close. */
let session = 0;
let refitSeq = 0;
/** The newest save queued. */
let saveSeq = 0;
let saveTimer: ReturnType<typeof setTimeout> | null = null;
/**
 * After a switch lands, how long a report may still show the version before: the player's
 * reports run at 30 Hz, so one sent just before the switch can arrive just after it.
 */
export const LISTEN_GRACE_MS = 150;
/**
 * The newest switch the user made and no report has shown yet. Until a report shows it (or the
 * grace after its command landed runs out), reports keep the user's choice on screen.
 */
let wanted: { listen: Listen; until: number } | null = null;

/** Whether a report's `listen` replaces the one shown, and the wanted switch it settles. */
function reportedListen(reported: Listen, now: number): boolean {
  if (wanted) {
    if (sameListen(reported, wanted.listen)) {
      wanted = null;
      return true;
    }
    if (now <= wanted.until) return false;
    // The player says otherwise past the grace: it is the authority.
    wanted = null;
  }
  return true;
}

function message(e: unknown): string {
  if (typeof e === 'object' && e !== null && 'message' in e)
    return String((e as { message: unknown }).message);
  return String(e);
}

const CLOSED = {
  fileId: null,
  opened: null,
  phase: 'closed',
  analysing: null,
  decoded: 0,
  decodeDone: false,
  error: null,
  edits: history(NO_EDIT),
  grid: null,
  fit: null,
  refitNote: null,
  confirmed: false,
  saving: false,
  saveError: null,
  click: true,
  player: IDLE_PLAYER,
  listen: OPEN_LISTEN,
  meter: null,
  overs: NO_OVERS,
  onsets: new Float64Array(0),
} satisfies Partial<TrackState>;

export const useTrack = create<TrackState>()((set, get) => {
  /** Refits the present edit and shows the answer, unless a newer edit was made meanwhile. */
  async function refit(): Promise<void> {
    const { fileId, edits, phase } = get();
    if (fileId === null || phase !== 'open') return;
    const seq = ++refitSeq;
    try {
      const fit = await gridRefit(fileId, edits.present);
      if (seq !== refitSeq) return;
      if (fit.header.grid === null) {
        // Keep the last grid on screen rather than none.
        set({ refitNote: 'This edit gives no grid. Undo it, or type the BPM.' });
      } else {
        set({ fit, grid: fit.header.grid, refitNote: null });
      }
    } catch (e) {
      if (seq === refitSeq) set({ refitNote: message(e) });
    }
  }

  /** Queues a save of `edit`; the table's row follows the answer. */
  function queueSave(fileId: number, edit: GridEdit, confirm: boolean): Promise<boolean> {
    const seq = ++saveSeq;
    const token = session;
    // Only the newest save, with no edit waiting after it, says what the view shows.
    const newest = () => session === token && seq === saveSeq && saveTimer === null;
    return enqueue(async () => {
      try {
        const update = await gridCommit(fileId, edit, confirm);
        useLibrary.getState().applyRowUpdate(update);
        if (newest()) set({ saving: false, saveError: null, confirmed: update.row.confirmed });
        return true;
      } catch (e) {
        if (session === token)
          set({ saveError: message(e), ...(newest() ? { saving: false } : {}) });
        return false;
      }
    });
  }

  /** Queues the save an edit is waiting for, now. */
  function flush(): void {
    if (saveTimer === null) return;
    clearTimeout(saveTimer);
    saveTimer = null;
    const { fileId, edits } = get();
    if (fileId !== null) void queueSave(fileId, edits.present, false);
  }

  function scheduleSave(): void {
    if (saveTimer !== null) clearTimeout(saveTimer);
    set({ saving: true, confirmed: false });
    saveTimer = setTimeout(() => {
      saveTimer = null;
      const { fileId, edits } = get();
      if (fileId !== null) void queueSave(fileId, edits.present, false);
    }, SAVE_DELAY_MS);
  }

  function changed(next: History, estimate?: Grid): void {
    set({ edits: next, ...(estimate ? { grid: estimate } : {}) });
    void refit();
    scheduleSave();
  }

  /** Sends a player command in queue order, while a track is open. */
  function playerCommand(command: () => Promise<void>): void {
    if (get().phase !== 'open') return;
    enqueue(command).catch((e: unknown) => set({ player: { ...get().player, error: message(e) } }));
  }

  return {
    ...CLOSED,

    readBins(samplesPerBin, firstBin, bins) {
      const { fileId } = get();
      if (fileId === null) return Promise.reject(new Error('no track is open'));
      return readPeaks(fileId, samplesPerBin, firstBin, bins);
    },

    async open(fileId) {
      flush();
      const token = ++session;
      refitSeq++;
      wanted = null;
      set({ ...CLOSED, fileId, phase: 'opening' });
      try {
        const opened = await enqueue(() =>
          trackOpen(fileId, (event) => {
            if (session === token) get().handle(event);
          }),
        );
        // A newer open or a close was issued meanwhile; it runs after this one in the queue.
        if (session !== token) return;
        set({
          opened,
          phase: 'open',
          analysing: null,
          grid: opened.grid,
          edits: history(opened.edit),
          confirmed: opened.confirmed,
        });
        void trackOnsets(fileId).then(
          (onsets) => session === token && set({ onsets }),
          () => {},
        );
        await refit();
      } catch (e) {
        if (session === token) set({ phase: 'failed', error: message(e) });
      }
    },

    async close() {
      flush();
      session++;
      refitSeq++;
      wanted = null;
      set({ ...CLOSED });
      await enqueue(() => trackClose()).catch(() => {});
    },

    edit(next, estimate, replace = false) {
      if (get().phase !== 'open') return;
      const h = get().edits;
      if (replace) {
        if (h.present !== next) changed({ ...h, present: next, future: [] }, estimate);
        return;
      }
      const pushed = push(h, next);
      if (pushed !== h) changed(pushed, estimate);
    },

    undo() {
      if (get().phase !== 'open') return;
      const h = undo(get().edits);
      if (h !== get().edits) changed(h);
    },

    redo() {
      if (get().phase !== 'open') return;
      const h = redo(get().edits);
      if (h !== get().edits) changed(h);
    },

    async confirm() {
      const { fileId, phase, edits } = get();
      if (fileId === null || phase !== 'open') return false;
      if (saveTimer !== null) {
        clearTimeout(saveTimer);
        saveTimer = null;
      }
      set({ saving: true });
      return queueSave(fileId, edits.present, true);
    },

    retrySave() {
      const { fileId, phase, edits } = get();
      if (fileId === null || phase !== 'open') return;
      set({ saving: true });
      void queueSave(fileId, edits.present, false);
    },

    setClick(on) {
      set({ click: on });
      playerCommand(() => playerSetClick(on));
    },

    play(from) {
      playerCommand(() => playerPlay(from ?? null));
    },

    pause() {
      playerCommand(() => playerPause());
    },

    seek(sample) {
      if (get().phase !== 'open') return;
      set({ player: { ...get().player, position: sample } });
      playerCommand(() => playerSeek(Math.max(0, Math.round(sample))));
    },

    setListen(listen) {
      const { fileId, phase, listen: before } = get();
      if (fileId === null || phase !== 'open' || sameListen(listen, before)) return;
      set({ listen });
      const token = session;
      const want = { listen, until: Infinity };
      wanted = want;
      playerCommand(async () => {
        try {
          await gridPlayerListen(fileId, listen);
          if (wanted === want) want.until = performance.now() + LISTEN_GRACE_MS;
        } catch (e) {
          if (wanted === want) wanted = null;
          // Still what the user hears: the version before.
          if (session === token && get().listen === listen) set({ listen: before });
          throw e;
        }
      });
    },

    clearOver(side) {
      if (get().overs[side]) set({ overs: { ...get().overs, [side]: false } });
    },

    handle(event) {
      switch (event.type) {
        case 'analysing':
          set({ analysing: event.fraction });
          break;
        case 'decoded':
          set({ decoded: event.frames });
          break;
        case 'ready':
          set({ decoded: event.frames, decodeDone: true });
          break;
        case 'failed':
          set({ decoded: event.frames, decodeDone: true, error: event.error.message });
          break;
        case 'player':
          // A track still opening is paused at its start: a report now is of the track before.
          if (get().phase !== 'open') break;
          set({
            player: {
              playing: event.playing,
              position: event.position,
              underruns: event.underruns,
              error: null,
            },
            meter: event.meter,
            overs: latched(get().overs, event.meter, useSettings.getState().ceiling),
            // A report from before the newest switch never undoes it.
            ...(reportedListen(event.listen, performance.now()) &&
            !sameListen(event.listen, get().listen)
              ? { listen: event.listen }
              : {}),
          });
          break;
        case 'playerError':
          set({ player: { ...get().player, playing: false, error: event.message } });
          break;
      }
    },
  };
});

/** Whether the present edit differs from the analysis. */
export function edited(state: TrackState): boolean {
  return !isEmpty(state.edits.present);
}
