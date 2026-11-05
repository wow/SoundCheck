import { render, screen } from '@testing-library/react';
import type { GridCheck, Meter } from '@/lib/ipc';
import { GridCheckBadge } from './ExportCells';
import { gridCheckChip } from './gridCheck';

const FOUR_FOUR: Meter = { beatsPerBar: 4, unit: 'quarter', grouping: [1, 1, 1, 1] };
const THREE_FOUR: Meter = { beatsPerBar: 3, unit: 'quarter', grouping: [1, 1, 1] };

const ALL: Record<GridCheck['result'], GridCheck> = {
  pass: { result: 'pass', offsetMs: 0.4, bpmDiff: 0 },
  advisory: { result: 'advisory', disagreement: { kind: 'offBy', offsetMs: 12.3, bpmDiff: 0 } },
  checkBar1: { result: 'checkBar1', offsetMs: 468.8, bpmDiff: 0, beats: 1 },
  offBy: { result: 'offBy', offsetMs: -12.3, bpmDiff: 0 },
  bpmDiffers: { result: 'bpmDiffers', bpmDiff: 0.5, found: 128.5 },
  meterDiffers: { result: 'meterDiffers', found: THREE_FOUR, exported: FOUR_FOUR },
  noGridFound: { result: 'noGridFound' },
  notChecked: { result: 'notChecked', reason: { type: 'oddMeter', meter: THREE_FOUR } },
};

/** The classes of a tone that says something is wrong. */
const ALARM = /\b(text|bg|border)-(err|warn|accent)\b|rgba\(248|rgba\(251/;

describe('the grid-check chip', () => {
  it('matches on result alone: each has its own label and tone', () => {
    const labels = Object.fromEntries(Object.entries(ALL).map(([k, v]) => [k, gridCheckChip(v)]));
    expect(labels.pass).toMatchObject({ tone: 'positive' });
    expect(labels.advisory).toMatchObject({ tone: 'neutral' });
    expect(labels.checkBar1).toMatchObject({ label: 'Check bar 1', tone: 'attention' });
    expect(labels.offBy).toMatchObject({ label: 'Off by 12 ms', tone: 'quiet' });
    expect(labels.bpmDiffers).toMatchObject({ label: 'BPM +0.50', tone: 'quiet' });
    expect(labels.meterDiffers).toMatchObject({ label: 'Meter 3/4', tone: 'quiet' });
    expect(labels.noGridFound).toMatchObject({ label: 'No grid found', tone: 'quiet' });
    expect(labels.notChecked).toMatchObject({ label: 'Not checked', tone: 'muted' });
  });

  it('never looks like a failure on an advisory: no warning or error style', () => {
    render(<GridCheckBadge check={ALL.advisory} />);
    const chip = screen.getByText(gridCheckChip(ALL.advisory).label);
    expect(chip.className).not.toMatch(ALARM);
    expect(chip).toHaveAttribute('data-tone', 'neutral');
  });

  it('advisory: says the confirmed grid was kept, with the numbers on hover, for every kind', () => {
    const cases: [GridCheck, RegExp][] = [
      [ALL.advisory, /12\.3 ms|12 ms/],
      [{ result: 'advisory', disagreement: { kind: 'bpmDiffers', bpmDiff: 0.5, found: 128.5 } }, /128\.50 BPM \(\+0\.50\)/],
      [{ result: 'advisory', disagreement: { kind: 'meterDiffers', found: THREE_FOUR } }, /3\/4/],
      [{ result: 'advisory', disagreement: { kind: 'noGridFound' } }, /no grid/],
    ];
    for (const [check, numbers] of cases) {
      const { text, tone } = gridCheckChip(check);
      expect(tone).toBe('neutral');
      expect(text).toContain('your confirmed grid was kept');
      expect(text).toMatch(numbers);
    }
    render(<GridCheckBadge check={ALL.advisory} />);
    expect(screen.getByText('Reads differently')).toHaveAttribute('title', expect.stringContaining('12'));
  });

  it('check bar 1 is amber and tells the count and direction from `beats`', () => {
    render(<GridCheckBadge check={ALL.checkBar1} />);
    expect(screen.getByText('Check bar 1')).toHaveClass('text-warn');
    expect(gridCheckChip(ALL.checkBar1).text).toContain('1 beat later');
    const earlier = gridCheckChip({ result: 'checkBar1', offsetMs: -937, bpmDiff: 0, beats: -2 });
    expect(earlier.text).toContain('2 beats earlier');
  });

  it('reports with the measured difference and never says it failed', () => {
    for (const check of [ALL.offBy, ALL.bpmDiffers, ALL.meterDiffers, ALL.noGridFound]) {
      const { text } = gridCheckChip(check);
      expect(text).toContain('The export succeeded');
      expect(text).not.toMatch(/fail|error/i);
    }
    expect(gridCheckChip(ALL.offBy).text).toContain('12 ms earlier');
    expect(gridCheckChip(ALL.bpmDiffers).text).toContain('128.50 BPM');
    expect(gridCheckChip(ALL.meterDiffers).text).toContain('exported as 4/4');
    render(<GridCheckBadge check={ALL.offBy} />);
    expect(screen.getByText('Off by 12 ms').className).not.toMatch(/\b(text|bg|border)-err\b/);
  });

  it('a muted reason for every way a file is not checked', () => {
    const reasons: Extract<GridCheck, { result: 'notChecked' }>['reason'][] = [
      { type: 'noGrid' },
      { type: 'gridWithheld' },
      { type: 'oddMeter', meter: THREE_FOUR },
      { type: 'xmlOnly' },
      { type: 'notAnalysed' },
    ];
    const texts = reasons.map((reason) => gridCheckChip({ result: 'notChecked', reason }).text);
    expect(new Set(texts).size).toBe(reasons.length);
    for (const text of texts) expect(text.length).toBeGreaterThan(10);
    render(<GridCheckBadge check={ALL.notChecked} />);
    expect(screen.getByText('Not checked')).toHaveClass('text-fg-2');
  });
});
