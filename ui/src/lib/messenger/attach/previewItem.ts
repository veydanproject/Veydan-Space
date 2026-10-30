// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A test item of the sheet of the paperclip: the proof that a new function is
// one file with an item in the registry, and the sheet and the composers do
// not change. Only the browser preview (`make dev`) and the tests load it
// (entry/mobile.ts, behind `import.meta.env.DEV`); no build of a product has it.

import { provideAttachAction } from '../seams/attach.svelte';

export function providePreviewAttach(): () => void {
  return provideAttachAction({
    id: 'preview',
    order: 40,
    icon: 'file',
    label: 'msg_attach_media',
    badge: 'new',
    produces: 'files',
    run: async () => ({ type: 'files', paths: ['/home/dev/Documents/preview-item.txt'], original: false }),
  });
}
