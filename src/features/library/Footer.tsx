import { useEffect, useState } from 'react';
import { appVersion } from '@/lib/ipc';
import { clock } from '@/lib/format';
import { useLibrary } from '@/state/library';
import { cancelAnalysis } from '@/features/pipeline/actions';

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

/** Progress, ETA and Cancel while a job runs; the last batch's summary otherwise. */
export function Footer({ cancelling }: { cancelling: boolean }) {
  const job = useLibrary((s) => s.job);
  const last = useLibrary((s) => s.lastBatch);
  const version = useVersion();
  const fraction = job && job.total > 0 ? job.done / job.total : 0;
  return (
    <footer className="flex h-[46px] shrink-0 items-center gap-4 border-t border-line bg-bg-1 px-[18px] text-xs text-fg-2">
      {/* Announced once when a job starts and once when it ends, never per progress step. */}
      <span className="sr-only" aria-live="polite">
        {job ? `Analysing ${job.total} tracks` : last ? 'Analysis finished' : ''}
      </span>
      {job ? (
        <>
          <span className="font-mono text-fg-1">
            Analysing {job.done} / {job.total}
            {job.etaMs !== null && ` · ETA ${clock(job.etaMs)}`}
            {job.realtimeX !== null && ` · ${Math.round(job.realtimeX)}× real time`}
          </span>
          <div
            className="h-1 w-48 overflow-hidden rounded-sm bg-bg-3"
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
            onClick={cancelAnalysis}
            disabled={cancelling}
            className="inline-flex h-7 items-center rounded-md border border-line px-2.5 text-[12.5px] font-medium text-fg-0 hover:bg-bg-2 disabled:opacity-45"
          >
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </button>
        </>
      ) : last ? (
        <span>
          Last batch: {last.analysed} analysed in {clock(last.seconds * 1000)}
          {last.review > 0 && `, ${last.review} need review`}
          {last.failed > 0 && `, ${last.failed} could not be read`}
          {last.cancelled && ' (cancelled)'}
        </span>
      ) : (
        <span>Open source · MIT OR Apache-2.0</span>
      )}
      <div className="flex-1" />
      <span className="font-mono" data-testid="version">
        {version ? `v${version}` : ''}
      </span>
    </footer>
  );
}
