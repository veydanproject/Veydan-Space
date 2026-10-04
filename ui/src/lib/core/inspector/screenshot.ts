// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get } from 'svelte/store';
import { theme } from '$lib/core/theme';
import { page } from '$app/stores';

function buildFilename(zoom: number): string {
  const route = get(page).url.pathname.replace(/\//g, '_').replace(/^_/, '') || 'home';
  const th = get(theme);
  const now = new Date();
  const ts = now.toISOString().replace(/[:.TZ-]/g, '').slice(0, 15);
  const w = window.innerWidth;
  const h = window.innerHeight;
  return `qa_${route}_${th}_${zoom}pct_${w}x${h}_${ts}.png`;
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function takeScreenshot(zoom: number): Promise<void> {
  const html2canvas = (await import('html2canvas')).default;

  const canvas = await html2canvas(document.body, {
    useCORS: true,
    allowTaint: true,
    scale: window.devicePixelRatio,
    ignoreElements: (el) => el.hasAttribute('data-html2canvas-ignore'),
  });

  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('toBlob failed'))), 'image/png');
  });

  const filename = buildFilename(zoom);

  if (isTauri) {
    const { save } = await import('@tauri-apps/plugin-dialog');
    const { writeFile } = await import('@tauri-apps/plugin-fs');

    const path = await save({
      defaultPath: filename,
      filters: [{ name: 'PNG Image', extensions: ['png'] }],
    });
    if (!path) return;

    const buffer = await blob.arrayBuffer();
    await writeFile(path, new Uint8Array(buffer));
  } else {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
}
