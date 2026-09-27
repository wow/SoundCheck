/**
 * Waveform bins fetched from the engine in tiles of 1,024 bins per level, each requested once
 * and kept in a small least-recently-used cache. A tile holding bins the decoder had not reached
 * yet is fetched again, when next drawn, after `refreshPartial` (called as decoding progresses);
 * it keeps serving what it has until the new bins arrive, so the waveform never blinks.
 */

export const TILE_BINS = 1024;
/** The engine's mark for a bin not decoded yet. */
export const NOT_DECODED = -32768;

export type FetchBins = (
  samplesPerBin: number,
  firstBin: number,
  bins: number,
) => Promise<Int16Array>;

interface Tile {
  samplesPerBin: number;
  index: number;
  data: Int16Array | null;
  partial: boolean;
  /** Partial and refreshed since it was asked for: fetched again when next drawn. */
  stale: boolean;
  loading: boolean;
  used: number;
}

export class PeakTiles {
  private tiles = new Map<string, Tile>();
  private clock = 0;
  /** Bumped by every refresh: an answer asked for before the latest one may be out of date. */
  private generation = 0;

  constructor(
    private readonly fetchBins: FetchBins,
    private readonly onLoad: () => void,
    private readonly capacity = 64,
  ) {}

  /** Min and max of bin `bin` at `samplesPerBin`, or null while its tile loads or is not decoded. */
  bin(samplesPerBin: number, bin: number): [number, number] | null {
    if (bin < 0) return null;
    const tileIndex = Math.floor(bin / TILE_BINS);
    const key = `${samplesPerBin}:${tileIndex}`;
    let tile = this.tiles.get(key);
    if (tile === undefined) {
      tile = {
        samplesPerBin,
        index: tileIndex,
        data: null,
        partial: false,
        stale: false,
        loading: false,
        used: 0,
      };
      this.tiles.set(key, tile);
      this.load(key, tile);
      this.evict();
    } else if (tile.stale && !tile.loading) {
      this.load(key, tile);
    }
    tile.used = ++this.clock;
    if (tile.data === null) return null;
    const k = 2 * (bin - tileIndex * TILE_BINS);
    const min = tile.data[k];
    const max = tile.data[k + 1];
    if (min === undefined || max === undefined || min === NOT_DECODED) return null;
    return [min, max];
  }

  /**
   * Marks tiles with undecoded bins to be fetched again. A tile whose answer is still on its way
   * is judged when it lands: if it has undecoded bins, it is fetched again too.
   */
  refreshPartial(): void {
    this.generation++;
    for (const tile of this.tiles.values()) if (tile.partial) tile.stale = true;
  }

  clear(): void {
    this.tiles.clear();
  }

  get size(): number {
    return this.tiles.size;
  }

  private load(key: string, tile: Tile): void {
    const asked = this.generation;
    tile.loading = true;
    tile.stale = false;
    this.fetchBins(tile.samplesPerBin, tile.index * TILE_BINS, TILE_BINS).then(
      (data) => {
        tile.loading = false;
        if (this.tiles.get(key) !== tile) return;
        tile.data = data;
        tile.partial = data.includes(NOT_DECODED);
        tile.stale = tile.partial && asked !== this.generation;
        this.onLoad();
      },
      () => {
        tile.loading = false;
        if (this.tiles.get(key) !== tile) return;
        // Asked for again when next drawn: a tile that never loaded from scratch, one with bins
        // keeping them meanwhile.
        if (tile.data === null) this.tiles.delete(key);
        else tile.stale = true;
      },
    );
  }

  private evict(): void {
    while (this.tiles.size > this.capacity) {
      let oldest: string | null = null;
      let used = Infinity;
      for (const [key, tile] of this.tiles) {
        if (tile.used < used) {
          used = tile.used;
          oldest = key;
        }
      }
      if (oldest === null) return;
      this.tiles.delete(oldest);
    }
  }
}
