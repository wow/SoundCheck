import { DEFAULT_SETTINGS, exportSettings, sanitize, savedPart, useSettings } from './settings';

const initial = useSettings.getState();
beforeEach(() => useSettings.setState(initial, true));

describe('export settings', () => {
  it('default to Prepare, in place with backup, source depth, XML on, TBPM in Prepare only', () => {
    const s = useSettings.getState();
    expect(exportSettings(s, 'prepare')).toEqual({
      batchMode: 'prepare',
      place: 'inPlace',
      depth: null,
      gridOnly: false,
      tbpm: true,
      xml: true,
      leadMs: 5,
    });
    expect(exportSettings(s, 'library').tbpm).toBe(false);
  });

  it('grid only keeps the source depth, whatever is set', () => {
    useSettings.getState().setDepth(24);
    expect(exportSettings(useSettings.getState(), 'prepare').depth).toBe(24);
    useSettings.getState().setGridOnly(true);
    expect(exportSettings(useSettings.getState(), 'prepare')).toMatchObject({ depth: null, gridOnly: true });
  });

  it('are in the saved part, so they persist', () => {
    const s = useSettings.getState();
    s.setBatchMode('library');
    s.setPlace('folder');
    s.setOutDir('/Music/Out');
    s.setDepth(16);
    s.setGridOnly(true);
    s.setTbpm(true, 'library');
    s.setXml(false);
    expect(savedPart(useSettings.getState())).toMatchObject({
      batchMode: 'library',
      place: 'folder',
      outDir: '/Music/Out',
      depth: 16,
      gridOnly: true,
      tbpm: { prepare: true, library: true },
      xml: false,
    });
  });

  it('a saved copy round-trips, and a damaged one keeps the defaults', () => {
    const s = useSettings.getState();
    s.setBatchMode('library');
    s.setDepth(24);
    s.setOutDir('/x');
    const saved = JSON.parse(JSON.stringify(savedPart(useSettings.getState()))) as Record<string, unknown>;
    expect(sanitize(saved)).toMatchObject({ batchMode: 'library', depth: 24, outDir: '/x' });
    expect(
      sanitize({
        batchMode: 'x',
        place: 3,
        outDir: '',
        depth: 20,
        gridOnly: 'yes',
        tbpm: { prepare: 1 },
        xml: null,
        seratoPrepareFolders: 'no',
      }),
    ).toEqual({});
    // A store from before these settings gets their defaults.
    useSettings.getState().hydrate(sanitize({ target: -9 }));
    expect(useSettings.getState()).toMatchObject({ batchMode: 'prepare', target: -9, tbpm: DEFAULT_SETTINGS.tbpm });
  });

  it('source depth saved as null survives; an absent one is not read as a choice', () => {
    expect(sanitize({ depth: null })).toEqual({ depth: null });
    expect(sanitize({})).toEqual({});
  });

  it('remembers the folders chosen for Prepare, and forgets them for Library', () => {
    const s = useSettings.getState();
    s.setBatchMode('prepare', ['/a', '/b']);
    expect(useSettings.getState().seratoPrepareFolders).toEqual(['/a', '/b']);
    s.setBatchMode('prepare', ['/a']);
    expect(useSettings.getState().seratoPrepareFolders).toEqual(['/b', '/a']);
    s.setBatchMode('library', ['/b']);
    expect(useSettings.getState()).toMatchObject({ batchMode: 'library', seratoPrepareFolders: ['/a'] });
  });

  it('caps the folders it remembers', () => {
    const many = Array.from({ length: 300 }, (_, i) => `/f${i}`);
    useSettings.getState().setBatchMode('prepare', many);
    expect(useSettings.getState().seratoPrepareFolders).toHaveLength(200);
    expect(useSettings.getState().seratoPrepareFolders.at(-1)).toBe('/f299');
    expect(sanitize({ seratoPrepareFolders: many }).seratoPrepareFolders).toHaveLength(200);
  });
});
