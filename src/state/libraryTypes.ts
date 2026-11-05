import type {
  ExportArtefacts,
  ExportPreviewRow,
  ExportRow,
  FileEntry,
  IpcError,
  JobEvent,
  Plan,
  Replan,
  RevertResult,
  RowAnalysis,
  RowUpdate,
  SessionSnapshot,
  XmlWritten,
} from '@/lib/ipc';

/** Where a row stands, as the Status column shows it. */
export type RowState =
  | 'queued'
  | 'analysing'
  | 'analysed'
  | 'needsReview'
  | 'skipped'
  | 'error'
  | 'cancelled'
  | 'exportQueued'
  | 'processing'
  | 'writing'
  | 'checking'
  | 'done'
  | 'failed';

export interface Row {
  entry: FileEntry;
  state: RowState;
  /** Analysis progress while analysing, 0..1. */
  progress: number;
  analysis?: RowAnalysis;
  plan?: Plan;
  /** The settings revision `plan` was decided with; a newer plan always wins. */
  planRevision?: number;
  error?: IpcError;
  /** The BPM range the row was last analysed with, to know when it is stale. */
  analysedRange?: [number, number];
  /** What its export does or did: the plan while it waits, then each step of the job. */
  export?: ExportRow;
  /** The export standing before the running one, which a failure or a cancel leaves in effect. */
  exportBefore?: ExportRow;
  /** Why its export failed; the file was not changed. */
  exportError?: IpcError;
  /** A change of the file made in place is in effect, which Revert can put back. */
  revertible?: boolean;
  /** Its grid was saved since its export: exporting it again would write something new. */
  exportStale?: boolean;
  /** Revert is running on this row (it takes about 10 s for a 6-minute track). */
  reverting?: boolean;
  revertError?: IpcError;
  /** What could not be restored or analysed after the last revert. */
  revertNotes?: string[];
  /** Why the rekordbox XML left it out. */
  xmlNotListed?: string;
}

export type Filter = 'all' | 'review' | 'short' | 'skipped' | 'done';

export type JobKind = 'analysis' | 'export';

export interface JobProgress {
  kind: JobKind;
  jobId: number | null;
  /** The rows this job was started with. */
  fileIds: number[];
  done: number;
  total: number;
  etaMs: number | null;
  realtimeX: number | null;
  startedAt: number;
}

export interface LastBatch {
  analysed: number;
  review: number;
  failed: number;
  cancelled: boolean;
  seconds: number;
}

export interface LastExport {
  written: number;
  xmlOnly: number;
  skipped: number;
  failed: number;
  cancelled: boolean;
  /** Time it took; unknown for one a reloaded window only found the end of. */
  seconds: number | null;
  /** The window reloaded during it, which stopped it. */
  stoppedByReload?: boolean;
}

/** Which of the footer's summaries is the newest. */
export type LastRun = JobKind | null;

/** Files of a running batch: its holds cover all of them, all or none. `all`: not known which. */
export type Locked = ReadonlySet<number> | 'all' | null;

export interface LibraryState {
  rows: Record<number, Row>;
  order: number[];
  filter: Filter;
  query: string;
  selected: number | null;
  job: JobProgress | null;
  lastBatch: LastBatch | null;
  lastExport: LastExport | null;
  lastRun: LastRun;
  /** The latest batch's rekordbox XML and grid report. */
  artefacts: ExportArtefacts | null;
  /** Why they could not be written; the files themselves are fine. */
  artefactsFailed: IpcError | null;
  /** Tracks the XML button's XML left out. */
  xmlLeftOut: number;
  /** The rekordbox XML button's command is running. */
  xmlWriting: boolean;
  /** The files of the running export, or of a job a reloaded window can only wait for. */
  locked: Locked;
  /** Why the last job could not start, until the next one does. */
  aborted: IpcError | null;
  abortedKind: JobKind;
  /** A problem to show above the table (for example, settings the engine refused). */
  notice: string | null;
  /** Something the user should know that is not a problem; dismissed by the user. */
  info: string | null;
  add(entries: FileEntry[]): void;
  /** Marks rows queued for a job about to start with `range`. */
  queue(fileIds: number[], range: [number, number]): void;
  /** Marks rows waiting for an export about to start, with what its confirm step planned. */
  beginExport(fileIds: number[], planned: ExportPreviewRow[]): void;
  setReverting(fileId: number): void;
  applyRevert(fileId: number, result: RevertResult): void;
  revertFailed(fileId: number, error: IpcError): void;
  setXmlWriting(writing: boolean): void;
  applyXml(written: XmlWritten): void;
  setXmlFailed(error: IpcError): void;
  /** The job a reloaded window found running has ended; what it did was read with `restore`. */
  windDownEnded(): void;
  applyEvent(event: JobEvent): void;
  /** A row whose grid edit was saved: its analysis and plan, as an analysed event would. */
  applyRowUpdate(update: RowUpdate): void;
  applyReplan(replan: Replan): void;
  /** Rebuilds the table from the engine's session after the window reloaded. */
  restore(snapshot: SessionSnapshot, range: [number, number]): void;
  setNotice(notice: string | null): void;
  setInfo(info: string | null): void;
  /** Empties the list (the engine forgets it too, through `clearList`). */
  clear(): void;
  setFilter(filter: Filter): void;
  setQuery(query: string): void;
  select(fileId: number | null): void;
  moveSelection(delta: number): void;
  /** Selects the next visible Needs review row after the selection, wrapping; returns it. */
  selectNextReview(): number | null;
}
