// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api', () => ({ messengerApi: { media: { grantAccess: async () => {} } } }));

/** A webview whose query of the camera answers `state`, and whose camera gives a stream or refuses. */
function webview(state: string, camera: () => Promise<MediaStream>) {
  vi.stubGlobal('navigator', {
    permissions: { query: async () => ({ state }) },
    mediaDevices: { getUserMedia: camera },
  });
}

const stream = { getTracks: () => [] } as unknown as MediaStream;

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe('cameraAllowed', () => {
  it('follows the query until the camera gave a picture', async () => {
    const { cameraAllowed } = await import('./capture');
    webview('granted', async () => stream);
    expect(await cameraAllowed()).toBe(true);
    webview('prompt', async () => stream);
    expect(await cameraAllowed()).toBe(false);
  });

  it('keeps a camera that gave a picture, though Android WebView answers «prompt» (ROO-481)', async () => {
    const { cameraAllowed, openPreview } = await import('./capture');
    webview('prompt', async () => stream);
    expect(await openPreview()).toBe(stream);
    expect(await cameraAllowed()).toBe(true);
  });

  it('forgets it when the camera refuses', async () => {
    const { cameraAllowed, openPreview } = await import('./capture');
    webview('prompt', async () => stream);
    await openPreview();
    webview('prompt', async () => { throw new DOMException('no', 'NotAllowedError'); });
    expect(await openPreview()).toBeNull();
    expect(await cameraAllowed()).toBe(false);
  });
});
