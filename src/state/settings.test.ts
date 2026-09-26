import { decideSettings, presetLabel, sanitize, useSettings } from './settings';

const initial = useSettings.getState();
beforeEach(() => useSettings.setState(initial, true));

describe('settings store', () => {
  it('switching mode selects that mode first preset', () => {
    useSettings.getState().setMode('streaming');
    const s = useSettings.getState();
    expect(decideSettings(s)).toEqual({ mode: 'streaming', target: -14, ceiling: -1, bpmRange: [70, 180] });
    expect(presetLabel(s)).toBe('Spotify');
    useSettings.getState().setPreset('apple');
    expect(useSettings.getState().target).toBe(-16);
  });

  it('editing the DJ target changes the persisted DJ target, not the preset', () => {
    useSettings.getState().setTarget(-10.04);
    let s = useSettings.getState();
    expect(s).toMatchObject({ preset: 'djTarget', target: -10, djTarget: -10 });
    useSettings.getState().setPreset('clubHot');
    useSettings.getState().setPreset('djTarget');
    s = useSettings.getState();
    expect(s.target).toBe(-10);
  });

  it('editing another preset or the ceiling makes it Custom', () => {
    useSettings.getState().setPreset('clubHot');
    useSettings.getState().setTarget(-9);
    expect(presetLabel(useSettings.getState())).toBe('Custom');
    expect(useSettings.getState().djTarget).toBe(-11);
    useSettings.getState().setPreset('djTarget');
    useSettings.getState().setCeiling(-1);
    expect(useSettings.getState().preset).toBe('custom');
  });

  it('calibration sets and selects the DJ target', () => {
    useSettings.getState().setMode('streaming');
    useSettings.getState().calibrate(-9.26);
    expect(useSettings.getState()).toMatchObject({ mode: 'dj', preset: 'djTarget', djTarget: -9.3, target: -9.3 });
  });

  it('rejects an empty or inverted BPM range', () => {
    useSettings.getState().setBpmRange([180, 70]);
    expect(useSettings.getState().bpmRange).toEqual([70, 180]);
    useSettings.getState().setBpmRange([80, 160]);
    expect(useSettings.getState().bpmRange).toEqual([80, 160]);
  });

  it('clamps the target and ceiling to their limits', () => {
    useSettings.getState().setCeiling(0.5);
    expect(useSettings.getState().ceiling).toBe(0);
    useSettings.getState().setTarget(-2);
    expect(useSettings.getState().target).toBe(-4);
    useSettings.getState().setTarget(-80);
    expect(useSettings.getState().target).toBe(-30);
  });

  it('keeps only valid saved fields', () => {
    expect(
      sanitize({ mode: 'dj', ceiling: 2, target: -9, bpmRange: [180, 70], preset: 'bogus', djTarget: 'x' }),
    ).toEqual({ mode: 'dj', target: -9 });
    expect(sanitize({ bpmRange: [80, 160], ceiling: -1 })).toEqual({ bpmRange: [80, 160], ceiling: -1 });
  });
});
