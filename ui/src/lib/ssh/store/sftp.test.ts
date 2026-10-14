// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A move between a local panel and a server deletes only the sources the
// backend reports as arrived whole; a folder onto a folder asks first.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { FileEntry, TransferDoneEvent } from '$lib/ssh/types';

const api = vi.hoisted(() => ({
  fs: {
    home: vi.fn(async () => '/home/u'),
    list: vi.fn(async (): Promise<unknown[]> => []),
    stat: vi.fn(async (): Promise<unknown> => null),
    delete: vi.fn(async () => {}),
  },
  sftp: {
    list: vi.fn(async (): Promise<unknown[]> => []),
    stat: vi.fn(async (): Promise<unknown> => null),
    delete: vi.fn(async () => {}),
    transferStart: vi.fn(async (): Promise<string> => 'tr-1'),
  },
}));
vi.mock('../api', () => ({ api }));

const { filesStore, joinLocal, parentLocal } = await import('./sftp.svelte');

function entry(path: string, isDir: boolean): FileEntry {
  const name = path.split('/').pop() ?? path;
  return {
    name,
    path,
    is_dir: isDir,
    is_symlink: false,
    size: 0,
    mtime: null,
    mode: 0,
    permissions: '',
    octal: '',
    owner: null,
    group: null,
  } as FileEntry;
}

const SOURCES = [entry('/home/u/A', true), entry('/home/u/B', false), entry('/home/u/C', true)];

function setUp() {
  const [local, remote] = filesStore.panels;
  local.source = { kind: 'local' };
  local.path = '/home/u';
  local.entries = SOURCES;
  local.selected = SOURCES.map((e) => e.path);
  local.loading = false;
  remote.source = { kind: 'remote', connectionId: 'c1' };
  remote.path = '/srv';
  remote.entries = [];
  remote.selected = [];
  remote.loading = false;
  filesStore.transfers = [];
}

function done(p: Partial<TransferDoneEvent>): TransferDoneEvent {
  return {
    transfer_id: 'tr-1',
    kind: 'upload',
    error: null,
    cancelled: false,
    files_done: 0,
    files_skipped: 0,
    complete_items: [],
    ...p,
  };
}

describe('a move to the other panel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setUp();
  });

  it('deletes only the items that arrived whole and counts the rest', async () => {
    await filesStore.moveToOtherPanel(0);
    expect(api.sftp.transferStart).toHaveBeenCalledOnce();

    await filesStore.finishTransfer(
      done({ files_done: 4, files_skipped: 1, complete_items: ['/home/u/A', '/home/u/C'] })
    );

    expect(api.fs.delete.mock.calls.map((c: unknown[]) => c[0])).toEqual(['/home/u/A', '/home/u/C']);
    const tr = filesStore.transfers[0];
    expect(tr.status).toBe('done');
    expect(tr.moveKept).toBe(1);
  });

  it('deletes nothing when nothing arrived whole, failed or was cancelled', async () => {
    await filesStore.moveToOtherPanel(0);
    await filesStore.finishTransfer(done({ files_skipped: 3 }));
    expect(api.fs.delete).not.toHaveBeenCalled();
    expect(filesStore.transfers[0].moveKept).toBe(3);

    for (const end of [{ error: 'boom' }, { cancelled: true }]) {
      setUp();
      await filesStore.moveToOtherPanel(0);
      await filesStore.finishTransfer(done({ ...end, complete_items: ['/home/u/A'] }));
    }
    expect(api.fs.delete).not.toHaveBeenCalled();
  });

  it('keeps a source whose delete failed in the count', async () => {
    api.fs.delete.mockRejectedValueOnce(new Error('busy'));
    await filesStore.moveToOtherPanel(0);
    await filesStore.finishTransfer(done({ complete_items: ['/home/u/A', '/home/u/B', '/home/u/C'] }));
    expect(api.fs.delete).toHaveBeenCalledTimes(3);
    expect(filesStore.transfers[0].moveKept).toBe(1);
  });

  it('asks before moving a folder onto a folder; a copy merges as before', async () => {
    const [local] = filesStore.panels;
    local.entries = [SOURCES[0]];
    local.selected = [SOURCES[0].path];
    api.sftp.stat.mockResolvedValue(entry('/srv/A', true));

    const moving = filesStore.moveToOtherPanel(0);
    await vi.waitFor(() => expect(filesStore.conflict).not.toBeNull());
    expect(filesStore.conflict?.isDir).toBe(true);
    filesStore.resolveConflict('skip', false);
    await moving;
    expect(api.sftp.transferStart).not.toHaveBeenCalled();

    await filesStore.copyToOtherPanel(0);
    expect(filesStore.conflict).toBeNull();
    expect(api.sftp.transferStart).toHaveBeenCalledWith('upload', 'c1', [
      { src_path: '/home/u/A', dst_path: '/srv/A', overwrite: false },
    ]);
    api.sftp.stat.mockResolvedValue(null);
  });

  it('shows a refused transfer in the strip instead of losing it', async () => {
    api.sftp.transferStart.mockRejectedValueOnce(new Error('Another transfer is already writing to /srv/A'));
    await filesStore.moveToOtherPanel(0);
    const tr = filesStore.transfers[0];
    expect(tr.status).toBe('error');
    expect(tr.error).toContain('/srv/A');
    expect(api.fs.delete).not.toHaveBeenCalled();
  });
});

describe('local paths on Windows', () => {
  it('go up to the root of the drive or the share, never to /', () => {
    expect(parentLocal('C:\\Users\\me')).toBe('C:\\Users');
    expect(parentLocal('C:\\Users')).toBe('C:\\');
    expect(parentLocal('C:\\')).toBe('C:\\');
    expect(parentLocal('C:/Users/me/')).toBe('C:/Users');
    expect(parentLocal('\\\\srv\\share\\dir')).toBe('\\\\srv\\share\\');
    expect(parentLocal('\\\\srv\\share')).toBe('\\\\srv\\share\\');
    // POSIX paths stay as they were.
    expect(parentLocal('/home/u')).toBe('/home');
    expect(parentLocal('/')).toBe('/');
  });

  it('join with the separator of the path', () => {
    expect(joinLocal('C:\\Users\\me', 'a.txt')).toBe('C:\\Users\\me\\a.txt');
    expect(joinLocal('C:\\', 'a.txt')).toBe('C:\\a.txt');
    expect(joinLocal('/home/u', 'a.txt')).toBe('/home/u/a.txt');
  });

  it('the up button of a local panel stops at the drive', async () => {
    api.fs.home.mockResolvedValueOnce('C:\\Users\\me');
    await filesStore.setSource(0, { kind: 'local' });
    expect(filesStore.panels[0].path).toBe('C:\\Users\\me');
    await filesStore.up(0);
    await filesStore.up(0);
    expect(filesStore.panels[0].path).toBe('C:\\');
    await filesStore.up(0);
    expect(filesStore.panels[0].path).toBe('C:\\');
    expect(api.fs.list).not.toHaveBeenCalledWith('/');
  });
});
