import { render, screen } from '@testing-library/react';
import { analysed, analysis, entry, plan } from '@/test/fixtures';
import { useLibrary } from '@/state/library';
import { useSettings } from '@/state/settings';
import { Table } from './Table';

const initialLibrary = useLibrary.getState();
const initialSettings = useSettings.getState();
beforeEach(() => {
  useLibrary.setState(initialLibrary, true);
  useSettings.setState(initialSettings, true);
});

describe('Table', () => {
  it('renders only the rows in view out of 1,000', () => {
    useLibrary.getState().add(Array.from({ length: 1000 }, (_, i) => entry(i + 1)));
    render(<Table />);
    // Header row plus at most 20 in view + 8 overscan before measuring.
    expect(screen.getAllByRole('row').length).toBeLessThanOrEqual(40);
    expect(screen.getByRole('grid', { name: 'Tracks' })).toHaveAttribute('aria-rowcount', '1001');
  });

  it('shows the action, the meter badge and the status of an analysed row', () => {
    const lib = useLibrary.getState();
    lib.add([entry(1, 'Skalonga')]);
    const grid = analysis().grid;
    if (!grid) throw new Error('fixture has a grid');
    lib.applyEvent(
      analysed(
        1,
        plan({
          gain: { type: 'gain', gainDb: 1.2, shortByLu: 1.8, truePeakAfter: -0.5 },
          status: 'needsReview',
          review: [{ type: 'drifts' }],
        }),
        analysis({
          grid: { ...grid, meter: '9/8 · 2+2+2+3', fourFour: false, verdict: 'drifts', residualMaxMs: 31, driftPpm: 180 },
        }),
      ),
    );
    render(<Table />);
    expect(screen.getByText('Skalonga')).toBeInTheDocument();
    expect(screen.getByText('Gain +1.2 dB')).toBeInTheDocument();
    expect(screen.getByText(/Short by 1\.8 LU: the ceiling is reached/)).toBeInTheDocument();
    expect(screen.getByText('9/8 · 2+2+2+3')).toBeInTheDocument();
    expect(screen.getByText('Needs review')).toBeInTheDocument();
    expect(screen.getByText(/Drifts: max 31 ms, \+180 ppm/)).toBeInTheDocument();
  });

  it('names the statistic in the loudness header', () => {
    useLibrary.getState().add([entry(1)]);
    const { rerender } = render(<Table />);
    expect(screen.getByRole('columnheader', { name: 'Loudness · S-P95' })).toBeInTheDocument();
    useSettings.getState().setMode('streaming');
    rerender(<Table />);
    expect(screen.getByRole('columnheader', { name: 'Loudness · Integrated' })).toBeInTheDocument();
  });
});
