import { useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';
import type { LoudnessMode } from '@/lib/ipc';
import { useLibrary } from '@/state/library';
import { PRESETS, presetLabel, useSettings, type PresetId } from '@/state/settings';
import { calibrateFromLibrary } from '@/features/pipeline/actions';

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h2 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-fg-2">{title}</h2>
      {children}
    </section>
  );
}

function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { id: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="group" aria-label={label} className="flex gap-0.5 rounded-lg border border-line bg-bg-1 p-[3px]">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          aria-pressed={value === o.id}
          onClick={() => onChange(o.id)}
          className={cn(
            'h-7 flex-1 rounded-md text-[12.5px] font-semibold',
            value === o.id ? 'bg-bg-3 text-fg-0 shadow-[0_1px_0_rgba(0,0,0,.4)]' : 'text-fg-2',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/**
 * A number field that commits on Enter or blur, so the table replans once per edit rather than
 * per keystroke.
 */
function NumberField({
  label,
  unit,
  value,
  onCommit,
  width = 116,
  digits = 1,
}: {
  label: string;
  unit: string;
  value: number;
  onCommit: (v: number) => void;
  width?: number;
  digits?: number;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft !== null) {
      const v = Number(draft.replace('−', '-'));
      if (Number.isFinite(v)) onCommit(v);
    }
    setDraft(null);
  };
  return (
    <label className="flex flex-col gap-1" style={{ width }}>
      <span className="text-[11px] text-fg-2">{label}</span>
      <span className="flex h-[30px] items-center rounded-md border border-line bg-bg-1 px-2 focus-within:border-accent">
        <input
          type="text"
          inputMode="decimal"
          value={draft ?? value.toFixed(digits)}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            if (e.key === 'Escape') {
              setDraft(null);
              e.currentTarget.blur();
            }
          }}
          className="w-full min-w-0 bg-transparent font-mono text-[13px] font-medium text-fg-0 outline-none"
        />
        <span className="whitespace-nowrap text-[11px] text-fg-2">{unit}</span>
      </span>
    </label>
  );
}

const MODES: { id: LoudnessMode; label: string }[] = [
  { id: 'dj', label: 'DJ' },
  { id: 'streaming', label: 'Streaming' },
];

/** The right rail: what the batch is levelled to, and the DJ app's BPM range. */
export function Rail() {
  const s = useSettings();
  const analysed = useLibrary((l) => Object.values(l.rows).some((r) => r.analysis?.shortTermP95 != null));
  return (
    <aside className="flex w-[300px] shrink-0 flex-col gap-[22px] overflow-y-auto border-l border-line bg-bg-1 px-[18px] pb-4 pt-[18px]">
      <Section title="Loudness">
        <Segmented label="Loudness mode" value={s.mode} options={MODES} onChange={s.setMode} />
        <label className="relative flex flex-col gap-1">
          <span className="sr-only">Preset</span>
          <svg
            className="pointer-events-none absolute right-2.5 top-[9px]"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="var(--sc-fg-2)"
            strokeWidth="1.75"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M6 9l6 6 6-6" />
          </svg>
          <select
            value={s.preset === 'custom' ? '' : s.preset}
            onChange={(e) => s.setPreset(e.target.value as Exclude<PresetId, 'custom'>)}
            className="h-8 w-full appearance-none rounded-[7px] border border-line bg-bg-1 pl-2.5 pr-8 text-[13px] text-fg-0"
            aria-label="Preset"
          >
            {s.preset === 'custom' && <option value="">{presetLabel(s)}</option>}
            {PRESETS[s.mode].map((p) => (
              <option key={p.id} value={p.id}>
                {p.label} · {s.mode === 'dj' ? 'S-P95' : 'Integrated'}
              </option>
            ))}
          </select>
        </label>
        <div className="flex gap-2.5">
          <NumberField label="Target" unit="LUFS" value={s.target} onCommit={s.setTarget} />
          <NumberField label="Ceiling" unit="dBTP" value={s.ceiling} onCommit={s.setCeiling} />
        </div>
        <p className="text-[11.5px] leading-[1.45] text-fg-2">
          Gain only. When the ceiling would be hit the row says <span className="text-accent">Short by X LU</span>.{' '}
          {s.mode === 'dj' && (
            <button
              type="button"
              className="text-accent hover:text-[#ffd27a] disabled:text-fg-2"
              disabled={!analysed}
              onClick={() => void calibrateFromLibrary()}
            >
              Calibrate from my library
            </button>
          )}
        </p>
      </Section>
      <Section title="Grid">
        <span className="-mb-1 text-[11px] text-fg-2">DJ app BPM range</span>
        <div className="flex items-end gap-2.5">
          <NumberField
            label="Lowest"
            unit="BPM"
            digits={0}
            value={s.bpmRange[0]}
            onCommit={(v) => s.setBpmRange([v, s.bpmRange[1]])}
          />
          <NumberField
            label="Highest"
            unit="BPM"
            digits={0}
            value={s.bpmRange[1]}
            onCommit={(v) => s.setBpmRange([s.bpmRange[0], v])}
          />
        </div>
        <p className="text-[11.5px] leading-[1.45] text-fg-2">
          The tempo octave is chosen inside this range first, as rekordbox does. Changing it marks tracks for Analyse.
        </p>
      </Section>
      <div className="flex-1" />
      <p className="flex items-center gap-2 text-[11.5px] text-fg-2">
        Originals untouched. Nothing is written in this version.
      </p>
    </aside>
  );
}
