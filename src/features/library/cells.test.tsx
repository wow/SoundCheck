import { render, screen } from '@testing-library/react';
import type { Row } from '@/state/library';
import { analysis, entry, plan } from '@/test/fixtures';
import { BpmCell, LoudnessCell } from './cells';

function row(a = analysis()): Row {
  return {
    entry: entry(1),
    state: 'analysed',
    progress: 1,
    analysis: a,
    plan: plan({ measured: -8.4 }),
  };
}

const aksak = analysis({
  grid: {
    ...analysis().grid!,
    bpm: 250,
    meter: '9/8 · 2+2+2+3',
    fourFour: false,
    verdict: 'staticWarn',
  },
});

describe('table cells', () => {
  it('puts the true peak under the loudness when there is no TP column', () => {
    const { rerender } = render(<LoudnessCell row={row()} target={-11} bar={false} />);
    expect(screen.queryByText(/^TP/)).toBeNull();
    rerender(
      <LoudnessCell
        row={row(analysis({ truePeak: -1.2 }))}
        target={-11}
        bar={false}
        tpCeiling={-0.5}
      />,
    );
    expect(screen.getByText('TP -1.2')).toHaveClass('text-fg-2');
    // Above the ceiling, in red as in the TP column.
    rerender(<LoudnessCell row={row()} target={-11} bar={false} tpCeiling={-0.5} />);
    expect(screen.getByText('TP -0.3')).toHaveClass('text-err');
  });

  it('shortens the meter in the compact layout and stacks the flags in the narrow one', () => {
    const { rerender, container } = render(<BpmCell row={row(aksak)} />);
    expect(screen.getByText('9/8 · 2+2+2+3')).toBeInTheDocument();
    rerender(<BpmCell row={row(aksak)} variant="compact" />);
    expect(screen.getByText('9/8')).toHaveAttribute('title', '9/8 · 2+2+2+3');
    rerender(<BpmCell row={row(aksak)} variant="stacked" />);
    // Two lines: the tempo, then the meter and "check".
    const lines = container.firstElementChild?.children;
    expect(lines?.length).toBe(2);
    expect(lines?.[0]).toHaveTextContent('250.00');
    expect(lines?.[1]).toHaveTextContent('9/8check');
    // A plain 4/4 grid has nothing for a second line.
    rerender(<BpmCell row={row()} variant="stacked" />);
    expect(container.firstElementChild?.children.length).toBeGreaterThan(0);
    expect(screen.getByText('128.00').parentElement).toBe(container.firstElementChild);
  });
});
