import { NOT_DECODED, PeakTiles, TILE_BINS } from './peaks';

function tileOf(first: number, decodedBins = TILE_BINS): Int16Array {
  const data = new Int16Array(2 * TILE_BINS);
  for (let b = 0; b < TILE_BINS; b++) {
    const known = b < decodedBins;
    data[2 * b] = known ? -((first + b) % 1000) : NOT_DECODED;
    data[2 * b + 1] = known ? (first + b) % 1000 : NOT_DECODED;
  }
  return data;
}

describe('waveform tiles', () => {
  it('fetches each tile once and serves its bins', async () => {
    const fetch = vi.fn(async (_spb: number, first: number) => tileOf(first));
    const loaded = vi.fn();
    const tiles = new PeakTiles(fetch, loaded);
    expect(tiles.bin(256, 5)).toBeNull();
    expect(tiles.bin(256, 900)).toBeNull();
    await vi.waitFor(() => expect(loaded).toHaveBeenCalled());
    expect(tiles.bin(256, 5)).toEqual([-5, 5]);
    expect(tiles.bin(256, 1030)).toBeNull();
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(fetch).toHaveBeenLastCalledWith(256, TILE_BINS, TILE_BINS);
    expect(tiles.bin(-1, -3)).toBeNull();
  });

  it('fetches a partly decoded tile again after a refresh', async () => {
    let decoded = 100;
    const fetch = vi.fn(async (_spb: number, first: number) => tileOf(first, decoded));
    const tiles = new PeakTiles(fetch, () => {});
    tiles.bin(64, 0);
    await vi.waitFor(() => expect(tiles.bin(64, 10)).toEqual([-10, 10]));
    expect(tiles.bin(64, 500)).toBeNull();
    decoded = TILE_BINS;
    tiles.refreshPartial();
    // While the fresh copy loads, the bins it had are still served: no blank frame.
    expect(tiles.bin(64, 10)).toEqual([-10, 10]);
    await vi.waitFor(() => expect(tiles.bin(64, 500)).toEqual([-500, 500]));
    expect(fetch).toHaveBeenCalledTimes(2);
    tiles.refreshPartial();
    tiles.bin(64, 500);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('fetches again a partial tile whose answer was in flight during a refresh', async () => {
    // Decoding reaches bin 100 before the request is read and ends while the answer travels:
    // the refresh cannot see the tile is partial yet, and there is no later refresh.
    let answer: (data: Int16Array) => void = () => {};
    const fetch = vi
      .fn<(spb: number, first: number, bins: number) => Promise<Int16Array>>()
      .mockImplementationOnce(() => new Promise((r) => (answer = r)))
      .mockImplementation(async (_spb, first) => tileOf(first));
    const loaded = vi.fn();
    const tiles = new PeakTiles(fetch, loaded);
    tiles.bin(64, 0);
    tiles.refreshPartial();
    answer(tileOf(0, 100));
    await vi.waitFor(() => expect(loaded).toHaveBeenCalledTimes(1));
    expect(tiles.bin(64, 10)).toEqual([-10, 10]);
    await vi.waitFor(() => expect(tiles.bin(64, 500)).toEqual([-500, 500]));
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('asks again for a tile whose refresh failed, keeping what it had', async () => {
    let decoded = 100;
    let fail = false;
    const fetch = vi.fn(async (_spb: number, first: number) => {
      if (fail) throw new Error('not open');
      return tileOf(first, decoded);
    });
    const tiles = new PeakTiles(fetch, () => {});
    tiles.bin(64, 0);
    await vi.waitFor(() => expect(tiles.bin(64, 10)).toEqual([-10, 10]));
    decoded = TILE_BINS;
    fail = true;
    tiles.refreshPartial();
    tiles.bin(64, 0);
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
    await Promise.resolve();
    expect(tiles.bin(64, 10)).toEqual([-10, 10]);
    fail = false;
    tiles.bin(64, 0);
    await vi.waitFor(() => expect(tiles.bin(64, 500)).toEqual([-500, 500]));
  });

  it('keeps at most its capacity, dropping the least recently used tile', async () => {
    const fetch = vi.fn(async (_spb: number, first: number) => tileOf(first));
    const tiles = new PeakTiles(fetch, () => {}, 2);
    tiles.bin(8, 0);
    tiles.bin(8, TILE_BINS);
    tiles.bin(8, 0);
    tiles.bin(8, 2 * TILE_BINS);
    expect(tiles.size).toBe(2);
    await vi.waitFor(() => expect(tiles.bin(8, 3)).toEqual([-3, 3]));
    expect(fetch).toHaveBeenCalledTimes(3);
  });
});
