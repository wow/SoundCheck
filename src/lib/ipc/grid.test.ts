import { describe, expect, it } from 'vitest';
import { parseGridFit } from './grid';

function fitBytes(lines: number, residuals: number[], extra = 0): ArrayBuffer {
  const header = { grid: null, firstLine: -1, lines, worstLine: 1, matched: 2, attacks: 2 };
  const json = new TextEncoder().encode(JSON.stringify(header));
  const bytes = new ArrayBuffer(4 + json.length + 4 * residuals.length + extra);
  const view = new DataView(bytes);
  view.setUint32(0, json.length, true);
  new Uint8Array(bytes, 4, json.length).set(json);
  residuals.forEach((r, i) => view.setFloat32(4 + json.length + 4 * i, r, true));
  return bytes;
}

describe('grid refit bytes', () => {
  it('split into the JSON header and the f32 residuals, NaN kept', () => {
    const header = { grid: null, firstLine: -1, lines: 3, worstLine: 1, matched: 2, attacks: 2 };
    const fit = parseGridFit(fitBytes(3, [1.5, Number.NaN, -3.25]));
    expect(fit.header).toEqual(header);
    expect(fit.residuals[0]).toBe(1.5);
    expect(fit.residuals[1]).toBeNaN();
    expect(fit.residuals[2]).toBe(-3.25);
  });

  it('refuse bytes of the wrong shape with a message', () => {
    expect(() => parseGridFit(new ArrayBuffer(2))).toThrow(/no header length/);
    const huge = new ArrayBuffer(8);
    new DataView(huge).setUint32(0, 1000, true);
    expect(() => parseGridFit(huge)).toThrow(/1000-byte header/);
    expect(() => parseGridFit(fitBytes(1, [1], 2))).toThrow(/f32 residuals/);
    expect(() => parseGridFit(fitBytes(3, [1, 2]))).toThrow(/2 residuals for 3 lines/);
  });
});
