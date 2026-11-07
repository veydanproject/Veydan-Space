// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger's way into the app's one confirmation (core/ui/confirm.svelte.ts,
// shown by the shell's ConfirmHost): the question's title is its action with a
// question mark («Выйти из группы?»), the text under it, «Отмена» and the verb.
// `icon` is the action's icon in the red tile of a dangerous question
// (`trash-2`, `logout`, `user-minus`, `user-cancel`…; default `alert-triangle`);
// `title` is a question's own title where it has one (`msg_chat_delete_title`).

import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';

class ConfirmStore {
  ask(text: string, confirmLabel: string, danger = false, icon?: string, title?: string): Promise<boolean> {
    return ask({
      title: title ?? confirmTitle(confirmLabel),
      message: text,
      confirmLabel,
      variant: danger ? 'danger' : 'primary',
      icon,
    });
  }
}

export const confirmStore = new ConfirmStore();
