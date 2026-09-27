import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { cn } from '@/lib/utils';

/** A key hint, shown next to its control. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        'rounded border border-line px-1 font-mono text-[10px] leading-[15px] text-fg-2',
        className,
      )}
    >
      {children}
    </span>
  );
}

/** The grid view's quiet button: text, an optional key hint, an optional pressed state. */
export function Btn({
  children,
  hint,
  hintClassName,
  pressed,
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  hint?: string;
  hintClassName?: string;
  pressed?: boolean;
}) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      className={cn(
        'inline-flex h-[30px] items-center gap-[7px] whitespace-nowrap rounded-[7px] border px-3 text-[13px] font-medium',
        'focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent-2 disabled:opacity-40',
        pressed
          ? 'border-[rgba(245,185,66,.45)] bg-[rgba(245,185,66,.12)] text-accent'
          : 'border-transparent text-fg-1 hover:bg-bg-2 hover:text-fg-0',
        className,
      )}
      {...rest}
    >
      {children}
      {hint && <Kbd className={hintClassName}>{hint}</Kbd>}
    </button>
  );
}

export type ChipTone = 'teal' | 'amber' | 'ok' | 'warn' | 'err' | 'plain';

const CHIP: Record<ChipTone, string> = {
  teal: 'text-accent-2 bg-[rgba(79,209,197,.10)] border-[rgba(79,209,197,.35)]',
  amber: 'text-accent bg-[rgba(245,185,66,.10)] border-[rgba(245,185,66,.35)]',
  ok: 'text-ok bg-[rgba(74,222,128,.10)] border-[rgba(74,222,128,.35)]',
  warn: 'text-warn bg-[rgba(251,191,36,.10)] border-[rgba(251,191,36,.35)]',
  err: 'text-err bg-[rgba(248,113,113,.10)] border-[rgba(248,113,113,.35)]',
  plain: 'text-fg-1 bg-transparent border-line',
};

export function Chip({
  tone = 'plain',
  mono = true,
  children,
}: {
  tone?: ChipTone;
  mono?: boolean;
  children: ReactNode;
}) {
  return (
    <span
      className={cn(
        'inline-flex h-5 items-center whitespace-nowrap rounded-[5px] border px-[7px] text-[11px] font-medium',
        mono ? 'font-mono' : 'font-sans',
        CHIP[tone],
      )}
    >
      {children}
    </span>
  );
}
