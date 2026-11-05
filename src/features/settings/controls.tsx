import type { ReactNode } from 'react';
import { cn } from '@/lib/utils';

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h2 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-fg-2">{title}</h2>
      {children}
    </section>
  );
}

export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled = false,
  fit = false,
}: {
  label: string;
  value: T;
  options: { id: T; label: string }[];
  onChange: (v: T) => void;
  disabled?: boolean;
  /** Each option as wide as its label needs, for labels of very different lengths. */
  fit?: boolean;
}) {
  return (
    <div role="group" aria-label={label} className="flex gap-0.5 rounded-lg border border-line bg-bg-1 p-[3px]">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          aria-pressed={value === o.id}
          disabled={disabled}
          onClick={() => onChange(o.id)}
          className={cn(
            'h-7 rounded-md text-[12.5px] font-semibold disabled:opacity-45',
            fit ? 'flex-auto whitespace-nowrap px-1.5' : 'flex-1',
            value === o.id ? 'bg-bg-3 text-fg-0 shadow-[0_1px_0_rgba(0,0,0,.4)]' : 'text-fg-2',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A setting that is on or off, on a native checkbox. */
export function Toggle({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className={cn('flex items-center gap-2 text-[12.5px] text-fg-1', disabled && 'opacity-45')}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="size-3.5 shrink-0 accent-[var(--sc-accent)]"
      />
      {label}
    </label>
  );
}
