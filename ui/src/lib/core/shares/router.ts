// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The way in for what another app shares into Veydan (`shares/api`): the
// same router as the links' (`core/links/router`), on the shell's event of
// shares. The shell says `shares://incoming` without the share; the reader
// (`ShareInbox`) takes it and hands it to the module that shows it.

import { isTauri, LinkRouter, tauriListen } from '../links/router';

export const SHARE_EVENT = 'shares://incoming';

export const shareRouter = new LinkRouter(isTauri ? tauriListen : null, SHARE_EVENT);

if (import.meta.hot) {
  import.meta.hot.dispose(() => shareRouter.stop());
}
