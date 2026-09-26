import { plan } from '@/test/fixtures';
import { actionText, level, peak } from './format';

describe('format', () => {
  it('never prints minus zero, and marks overs with a plus', () => {
    expect(level(-0.04)).toBe('0.0');
    expect(level(-0.3)).toBe('-0.3');
    expect(peak(-0.04)).toBe('0.0');
    expect(peak(0.6)).toBe('+0.6');
    expect(peak(-1.2)).toBe('-1.2');
    expect(peak(null)).toBe('—');
  });

  it('says there is no boost when the peak already sits at the ceiling', () => {
    const a = actionText(plan({ gain: { type: 'gain', gainDb: 0, shortByLu: 2, truePeakAfter: -0.3 } }));
    expect(a).toMatchObject({ main: 'No boost: peak at the ceiling', detail: 'Short by 2.0 LU', tone: 'accent' });
    const b = actionText(plan({ gain: { type: 'gain', gainDb: 1.7, shortByLu: 3.9, truePeakAfter: -0.5 } }));
    expect(b.main).toBe('Gain +1.7 dB');
    expect(b.detail).toBe('Short by 3.9 LU: the ceiling is reached');
  });
});
