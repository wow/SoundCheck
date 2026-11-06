import { useEffect, useState } from 'react';
import { Loader2, X } from 'lucide-react';
import type { PendingChange } from '@/lib/ipc';
import { askForget, dismissForgotten, dismissRecovery, useRecovery } from '@/state/recovery';
import { ForgetConfirm } from './ForgetConfirm';
import {
  failedText,
  fileName,
  forgetRefusedText,
  forgottenText,
  PENDING_TODO,
  pendingHead,
  pendingReason,
  recoveredText,
  RUNNING_DETAIL,
  RUNNING_TEXT,
} from './text';

/** A check that ends this soon after start is not worth a line that flashes and goes. */
export const RUNNING_SHOW_AFTER_MS = 400;

const BAR = 'flex shrink-0 items-start gap-2 border-b px-4 py-2 text-[12.5px] leading-[1.45]';
const WARN = `${BAR} border-warn/45 bg-warn/10`;
const ERR = `${BAR} border-err/45 bg-err/10`;
const CALM = `${BAR} border-line bg-bg-2`;
const BUTTON =
  'inline-flex h-6 shrink-0 items-center whitespace-nowrap rounded-md border border-line px-2 text-[12px] font-medium text-fg-0 hover:bg-bg-3 disabled:cursor-default disabled:opacity-45';

/** `on`, once it has stayed on for `ms`. */
function useAfter(on: boolean, ms: number): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!on) return;
    const timer = setTimeout(() => setShown(true), ms);
    return () => {
      clearTimeout(timer);
      setShown(false);
    };
  }, [on, ms]);
  return on && shown;
}

function Dismiss({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className="ml-auto inline-flex size-6 shrink-0 items-center justify-center rounded text-fg-2 hover:text-fg-0"
    >
      <X className="size-3.5" aria-hidden="true" />
    </button>
  );
}

/** One change left pending: its file, why, and Forget. */
function Pending({ change }: { change: PendingChange }) {
  const forgetting = useRecovery((r) => r.forgetting);
  const error = useRecovery((r) =>
    r.forgetError?.txn === change.txn ? r.forgetError.error : null,
  );
  const reason = pendingReason(change.reason);
  const refused = error ? forgetRefusedText(error) : null;
  return (
    <li className="flex min-w-0 flex-col gap-0.5 py-1">
      <div className="flex min-w-0 items-center gap-2">
        <span
          className="min-w-0 max-w-[45%] shrink-0 truncate font-medium text-fg-0"
          title={change.path}
        >
          {fileName(change.path)}
        </span>
        <span className="min-w-0 flex-1 truncate text-fg-1" title={reason}>
          {reason}
        </span>
        <button
          type="button"
          className={BUTTON}
          disabled={forgetting !== null}
          aria-label={`Forget the change to ${fileName(change.path)}`}
          onClick={() => askForget(change)}
        >
          {forgetting === change.txn ? 'Forgetting…' : 'Forget'}
        </button>
      </div>
      {refused && (
        <p role="alert" className="break-words text-[12px] text-err">
          {refused}
        </p>
      )}
    </li>
  );
}

/** The line about what the last Forget kept: every file by its path, so each can be found. */
function Forgotten() {
  const forgotten = useRecovery((r) => r.forgotten);
  if (!forgotten) return null;
  const { head, kept } = forgottenText(forgotten);
  return (
    <div className={CALM}>
      <div className="flex max-h-[112px] min-w-0 flex-1 flex-col overflow-y-auto text-fg-1">
        <p>{head}</p>
        {kept.length > 0 && (
          <ul aria-label="Kept" className="flex flex-col">
            {kept.map((file) => (
              <li key={file.path} className="break-words text-[12px] text-fg-2">
                <span className="break-all font-mono text-fg-1">
                  {file.path}
                </span>
                : {file.what}
              </li>
            ))}
          </ul>
        )}
      </div>
      <Dismiss label="Dismiss" onClick={dismissForgotten} />
    </div>
  );
}

/** What is on screen, as one sentence for the live region that says it. */
function useAnnouncement(running: boolean): string {
  const status = useRecovery((r) => r.status);
  const dismissed = useRecovery((r) => r.dismissed);
  const forgotten = useRecovery((r) => r.forgotten);
  if (running) return RUNNING_TEXT;
  if (status?.state !== "finished") return "";
  const said = [
    forgotten ? forgottenText(forgotten).head : null,
    status.pending.length > 0 ? pendingHead(status.pending.length) : null,
    !dismissed || status.pending.length > 0
      ? recoveredText(status.recovered)
      : null,
  ];
  return said.filter(Boolean).join(" ");
}

/** The banner's lines, drawn; what they say is announced by the live region beside them. */
function Lines({ running }: { running: boolean }) {
  const status = useRecovery((r) => r.status);
  const dismissed = useRecovery((r) => r.dismissed);
  const asking = useRecovery((r) => r.asking);
  if (running) {
    return (
      <div className={CALM}>
        <Loader2
          className="mt-[3px] size-3.5 shrink-0 text-fg-1 motion-safe:animate-spin"
          aria-hidden="true"
        />
        <p className="min-w-0 flex-1 truncate text-fg-1" title={RUNNING_DETAIL}>
          <span className="text-fg-0">{RUNNING_TEXT}</span> {RUNNING_DETAIL}
        </p>
      </div>
    );
  }
  // The only reason it skips is that there is no home folder: then nothing can have been
  // interrupted either, and nothing would help to say.
  if (!status || status.state === "skipped" || status.state === "running")
    return null;
  if (status.state === "failed") {
    if (dismissed) return null;
    const { what, why, todo } = failedText(status.message);
    return (
      <div role="alert" className={ERR}>
        <div className="min-w-0 flex-1 break-words">
          <p className="font-medium text-err">{what}</p>
          <p className="text-fg-1">{`${why}.`}</p>
          <p className="text-fg-2">{todo}</p>
        </div>
        <Dismiss label="Dismiss" onClick={dismissRecovery} />
      </div>
    );
  }
  const recovered = recoveredText(status.recovered);
  const paths = status.recovered.map((r) => r.path).join("\n");
  if (status.pending.length === 0) {
    return (
      <>
        <Forgotten />
        {recovered && !dismissed && (
          <div className={CALM}>
            <p className="min-w-0 flex-1 truncate text-fg-1" title={paths}>
              {recovered}
            </p>
            <Dismiss label="Dismiss" onClick={dismissRecovery} />
          </div>
        )}
      </>
    );
  }
  return (
    <>
      <Forgotten />
      <section
        role="region"
        aria-label="Changes from the last session"
        className={WARN}
      >
        <div className="flex min-w-0 flex-1 flex-col">
          <p className="font-medium text-warn">
            {pendingHead(status.pending.length)}
          </p>
          <p className="text-fg-2">{PENDING_TODO}</p>
          <ul className="max-h-[92px] min-w-0 overflow-y-auto">
            {status.pending.map((change) => (
              <Pending key={change.txn} change={change} />
            ))}
          </ul>
          {recovered && (
            <p className="truncate text-fg-2" title={paths}>
              {recovered}
            </p>
          )}
        </div>
      </section>
      {asking && (
        <ForgetConfirm change={asking} onClose={() => askForget(null)} />
      )}
    </>
  );
}

/**
 * What the start-up recovery found, above the table: the check running, changes it could not
 * finish (each with Forget), changes it finished, or that it could not run. In the page's flow,
 * never over the list; a long list scrolls inside its own few lines. One live region is there
 * from the start and only its text changes, so screen readers say each new line.
 */
export function RecoveryBanner() {
  const status = useRecovery((r) => r.status);
  const running = useAfter(status?.state === "running", RUNNING_SHOW_AFTER_MS);
  const announcement = useAnnouncement(running);
  return (
    <>
      <div
        role="status"
        aria-live="polite"
        className="sr-only"
        data-testid="recovery-live"
      >
        {announcement}
      </div>
      <Lines running={running} />
    </>
  );
}
