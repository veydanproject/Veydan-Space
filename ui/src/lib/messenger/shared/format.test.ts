// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { fileIcon } from './format';

describe('fileIcon', () => {
  it('knows a file by its type, then by its extension', () => {
    for (const [name, mime, icon] of [
      ['a.png', 'image/png', 'image'],
      ['clip.mp4', 'video/mp4', 'video'],
      ['song.mp3', 'audio/mpeg', 'mic'],
      ['tsconfig.json', 'application/octet-stream', 'file-json'],
      ['backup.tar.gz', 'application/gzip', 'archive'],
      ['src.ZIP', '', 'archive'],
      ['report.xlsx', '', 'table-2'],
      ['data.csv', 'text/csv', 'table-2'],
      ['vite.config.js', 'text/javascript', 'file-code'],
      ['dev.sh', 'application/x-sh', 'file-code'],
      ['svelte.config.js', '', 'file-code'],
      ['spec.pdf', 'application/pdf', 'file-text'],
      ['notes.txt', 'text/plain', 'file-text'],
      ['letter', 'application/msword', 'file-text'],
      ['firmware.bin', 'application/octet-stream', 'file'],
      ['README', '', 'file'],
    ] as const) {
      expect(fileIcon(name, mime), name).toBe(icon);
    }
  });
});
