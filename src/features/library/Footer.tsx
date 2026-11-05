import { useEffect, useState } from 'react';
import { Loader2 } from 'lucide-react';
import { appVersion } from '@/lib/ipc';
import { clock } from '@/lib/format';
import { NARROW, useMediaQuery } from '@/lib/media';
import { useLibrary, xmlRowIds } from '@/state/library';
import { useRecovery } from '@/state/recovery';
import { cancelRunningJob } from '@/features/pipeline/actions';
import { reveal, writeXmlOfAnalysed } from '@/features/export/actions';
import { lastExportText, xmlListedText } from '@/features/export/text';

function useVersion(): string {
  const [version, setVersion] = useState('');
  useEffect(() => {
    let live = true;
    appVersion()
      .then((v) => live && setVersion(v))
      .catch(() => live && setVersion('dev'));
    return () => {
      live = false;
    };
  }, []);
  return version;
}

const LINK = 'shrink-0 whitespace-nowrap text-accent-2 hover:underline';

/**
 * A batch's links, each showing the file in the Finder: the rekordbox XML to import, the grid
 * report, and their folder. Shorter labels in a narrow window, where the full names do not fit.
 */
function ArtefactLinks({ narrow }: { narrow: boolean }) {
  const artefacts = useLibrary((s) => s.artefacts);
  if (!artefacts) return null;
  return (
    <nav aria-label="This batch's files" className="flex shrink-0 items-center gap-3">
      {artefacts.xml && (
        <button
          type="button"
          className={LINK}
          aria-label="Show soundcheck-rekordbox.xml in the Finder"
          title={artefacts.xml}
          onClick={() => reveal(artefacts.xml as string)}
        >
          {narrow ? 'XML' : 'soundcheck-rekordbox.xml'}
        </button>
      )}
      {artefacts.report && (
        <button
          type="button"
          className={LINK}
          aria-label="Show grid-report.csv in the Finder"
          title={artefacts.report}
          onClick={() => reveal(artefacts.report as string)}
        >
          {narrow ? 'CSV' : 'grid-report.csv'}
        </button>
      )}
      <button
        type="button"
        className={LINK}
        aria-label="Show in Finder"
        title={artefacts.folder}
        onClick={() => reveal(artefacts.folder)}
      >
        {narrow ? 'Finder' : 'Show in Finder'}
      </button>
    </nav>
  );
}

/** Writes a rekordbox XML of every analysed row, an exported one where its export is. */
function XmlButton({ idle }: { idle: boolean }) {
  const count = useLibrary((s) => xmlRowIds(s).length);
  const writing = useLibrary((s) => s.xmlWriting);
  const recovering = useRecovery((r) => r.running);
  const reason = recovering
    ? 'Checking for changes a crash interrupted.'
    : !idle
      ? 'A job is running.'
      : count === 0
        ? 'No track is analysed yet.'
        : `Writes an XML of ${count} analysed ${count === 1 ? 'track' : 'tracks'}; the files are not touched.`;
  return (
    <button
      type="button"
      onClick={() => void writeXmlOfAnalysed()}
      disabled={!idle || count === 0 || writing || recovering}
      title={reason}
      className="inline-flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md border border-line px-2.5 text-[12.5px] font-medium text-fg-0 hover:bg-bg-2 disabled:cursor-default disabled:opacity-45"
    >
      {writing && <Loader2 className="size-3 motion-safe:animate-spin" aria-hidden="true" />}
      rekordbox XML
    </button>
  );
}

/** Progress, ETA and Cancel while a job runs; the last batch's summary and files otherwise. */
export function Footer({
  cancelling,
  windingDown = false,
  idle = true,
}: {
  cancelling: boolean;
  windingDown?: boolean;
  idle?: boolean;
}) {
  const job = useLibrary((s) => s.job);
  const last = useLibrary((s) => s.lastBatch);
  const lastExport = useLibrary((s) => s.lastExport);
  const lastRun = useLibrary((s) => s.lastRun);
  const artefacts = useLibrary((s) => s.artefacts);
  const xmlLeftOut = useLibrary((s) => s.xmlLeftOut);
  const artefactsFailed = useLibrary((s) => s.artefactsFailed);
  const narrow = useMediaQuery(NARROW);
  const version = useVersion();
  const fraction = job && job.total > 0 ? job.done / job.total : 0;
  const doing = job?.kind === 'export' ? 'Exporting' : 'Analysing';
  const summary =
    lastRun === 'export' && lastExport
      ? lastExportText(lastExport)
      : lastRun === 'analysis' && last
        ? [
            `Last batch: ${last.analysed} analysed in ${clock(last.seconds * 1000)}`,
            last.review > 0 ? `, ${last.review} need review` : '',
            last.failed > 0 ? `, ${last.failed} could not be read` : '',
            last.cancelled ? ' (cancelled)' : '',
          ].join('')
        : null;
  const xml = artefacts ? xmlListedText(artefacts, xmlLeftOut) : null;
  const told = [summary, xml, artefactsFailed ? 'rekordbox XML not written' : null]
    .filter(Boolean)
    .join(' · ');
  return (
    <footer className="flex h-[46px] shrink-0 items-center gap-4 border-t border-line bg-bg-1 px-[18px] text-xs text-fg-2">
      {/* Announced once when a job starts and once when it ends, never per progress step. */}
      <span className="sr-only" aria-live="polite">
        {job
          ? `${doing} ${job.total} tracks`
          : windingDown
            ? 'Waiting for the job that was running'
            : lastRun === 'export'
              ? 'Export finished'
              : last
                ? 'Analysis finished'
                : ''}
      </span>
      {job ? (
        <>
          <span className="truncate font-mono text-fg-1">
            {doing} {job.done} / {job.total}
            {job.etaMs !== null && ` · ETA ${clock(job.etaMs)}`}
            {job.realtimeX !== null && ` · ${Math.round(job.realtimeX)}× real time`}
          </span>
          <div
            className="h-1 w-48 min-w-12 shrink-[2] overflow-hidden rounded-sm bg-bg-3"
            role="progressbar"
            aria-label="Batch progress"
            aria-valuemin={0}
            aria-valuemax={job.total}
            aria-valuenow={job.done}
          >
            <div className="h-full bg-accent-2" style={{ width: `${fraction * 100}%` }} />
          </div>
          <button
            type="button"
            onClick={cancelRunningJob}
            disabled={cancelling}
            className="inline-flex h-7 shrink-0 items-center rounded-md border border-line px-2.5 text-[12.5px] font-medium text-fg-0 hover:bg-bg-2 disabled:opacity-45"
          >
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </button>
        </>
      ) : windingDown ? (
        <span className="flex min-w-0 items-center gap-2 text-fg-1">
          <Loader2 className="size-3 shrink-0 motion-safe:animate-spin" aria-hidden="true" />
          <span className="truncate">The window reloaded during a job; waiting for it to stop…</span>
        </span>
      ) : told ? (
        <>
          <span className="min-w-0 truncate" title={told}>
            {told}
          </span>
          <ArtefactLinks narrow={narrow} />
        </>
      ) : (
        <span className="truncate">Open source · MIT OR Apache-2.0</span>
      )}
      <div className="flex-1" />
      {/* While a job runs it cannot be pressed, and a narrow footer needs the room for progress. */}
      {!(narrow && job) && <XmlButton idle={idle} />}
      <span className="shrink-0 font-mono" data-testid="version">
        {version ? `v${version}` : ''}
      </span>
    </footer>
  );
}
