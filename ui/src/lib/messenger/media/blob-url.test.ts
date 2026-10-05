// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { dataUrlToBlob } from './blob-url';

describe('dataUrlToBlob', () => {
  it('keeps the type and the bytes', async () => {
    const blob = dataUrlToBlob(`data:audio/webm;base64,${btoa('\x00\x01\xfe\xff')}`);
    expect(blob?.type).toBe('audio/webm');
    expect([...new Uint8Array(await blob!.arrayBuffer())]).toEqual([0, 1, 254, 255]);
  });

  it('refuses what is not a base64 data url', () => {
    expect(dataUrlToBlob('blob:http://x/1')).toBeNull();
    expect(dataUrlToBlob('data:text/plain,hello')).toBeNull();
    expect(dataUrlToBlob('data:audio/webm;base64,***')).toBeNull();
  });
});
