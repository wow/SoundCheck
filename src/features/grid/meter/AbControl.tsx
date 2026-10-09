import { EqualApproximately } from 'lucide-react';
import type { Version } from '@/lib/ipc';
import { signed } from '@/lib/format';
import { cn } from '@/lib/utils';
import { toggleMatch } from '../actions';
import { useTrack } from '../store';
import { TRANSPORT_FOLD } from '../fold';
import { Btn, Kbd } from '../ui';

export const MATCH_TIP =
  'Same level: with gain only the versions are identical; useful once the limiter changes the sound';

/**
 * What the player plays: the original or the processed version (`B`), at the same position;
 * and `Match level` (`Shift+B`), which plays both at the same loudness.
 */
export function AbControl() {
  const listen = useTrack((s) => s.listen);
  const setListen = useTrack((s) => s.setListen);
  const gain = useTrack((s) => s.opened?.gain ?? 0);
  const open = useTrack((s) => s.phase === 'open');
  const choose = (version: Version) => setListen({ ...listen, version });
  return (
    <>
      <div
        role="group"
        aria-label="Version heard"
        aria-keyshortcuts="B"
        title="Switch the version heard · B"
        className="inline-flex h-[30px] items-center rounded-[7px] border border-line p-[2px]"
      >
        <Segment
          on={listen.version === 'original'}
          disabled={!open}
          onClick={() => choose('original')}
        >
          Original
        </Segment>
        <Segment
          on={listen.version === 'processed'}
          disabled={!open}
          onClick={() => choose('processed')}
          aria-label={`Processed ${signed(gain)} dB`}
        >
          Processed <span className="font-mono text-[12px] tabular-nums">{signed(gain)} dB</span>
        </Segment>
      </div>
      <Kbd className={cn('ml-0.5', TRANSPORT_FOLD.hint)}>B</Kbd>
      <Btn
        hint="⇧B"
        hintClassName={TRANSPORT_FOLD.hint}
        pressed={listen.matched}
        disabled={!open}
        onClick={toggleMatch}
        aria-label="Match level"
        aria-keyshortcuts="Shift+B"
        title={MATCH_TIP}
        className="ml-1"
      >
        <EqualApproximately className="size-3.5 shrink-0" aria-hidden />
        <span className={TRANSPORT_FOLD.compact}>Match level</span>
      </Btn>
    </>
  );
}

function Segment({
  on,
  children,
  className,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { on: boolean }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      className={cn(
        'inline-flex h-[24px] items-center gap-1 whitespace-nowrap rounded-[5px] px-2.5 text-[12.5px] font-medium @max-[900px]:px-1.5',
        'focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2 disabled:opacity-40',
        on ? 'bg-[rgba(245,185,66,.14)] text-accent' : 'text-fg-1 hover:bg-bg-2 hover:text-fg-0',
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}
