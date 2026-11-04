import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it } from 'vitest';
import { exportFiles, forgetPending, revertFile, writeRekordboxXml } from './index';
import type { ExportSettings } from './index';

const settings: ExportSettings = {
  batchMode: 'prepare',
  place: 'inPlace',
  depth: null,
  gridOnly: false,
  tbpm: true,
  xml: true,
  leadMs: 5,
};

describe('export commands', () => {
  afterEach(() => clearMocks());

  it('reach their Rust commands with the arguments those take', async () => {
    const calls: [string, Record<string, unknown>][] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args as Record<string, unknown>]);
      return cmd === 'export' ? 7 : {};
    });
    const req = { fileIds: [1, 2], settings, outDir: null };
    await expect(exportFiles(req, () => undefined)).resolves.toBe(7);
    await revertFile(3);
    await writeRekordboxXml({ fileIds: [1], out: null });
    await forgetPending('6ac5b4f4-1-0');

    expect(calls.map(([cmd]) => cmd)).toEqual(['export', 'revert_file', 'write_rekordbox_xml', 'forget_pending']);
    expect(calls[0]?.[1]).toMatchObject({ req });
    expect(calls[0]?.[1].onEvent).toBeDefined();
    expect(calls[1]?.[1]).toEqual({ fileId: 3 });
    expect(calls[2]?.[1]).toEqual({ req: { fileIds: [1], out: null } });
    expect(calls[3]?.[1]).toEqual({ txn: '6ac5b4f4-1-0' });
  });

  it('reject with the typed refusal', async () => {
    const refusal = { kind: 'recoveryRunning', message: 'running', fileId: null };
    mockIPC(() => {
      throw refusal;
    });
    await expect(exportFiles({ fileIds: [1], settings, outDir: null }, () => undefined)).rejects.toEqual(refusal);
  });
});
