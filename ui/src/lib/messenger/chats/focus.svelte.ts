// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { page } from '$app/state';
import { messengerApi } from '../api';
import { BASE } from '../mobile/routes';
import { chatStore, inSight, lockSight } from './chatStore.svelte';
import { focusOf, focusToSay } from './focus';

/** What the user does in the window: each renews the word of the chat read. */
const TOUCHES = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart'] as const;

/**
 * Tells the runtime which chat is being read here (ROO-1275): at once when
 * the chat opens, changes or closes, the window gains or loses the focus,
 * the page is hidden or the lock closes; again while the user does
 * something in it (a pointer, a key), every half minute at most. A page
 * left without a touch says nothing, and the runtime lets its word go
 * after five minutes. Returns what undoes it.
 */
export function focusLease(): () => void {
  let said: string | null = null;
  let saidAt = 0;
  const now = () =>
    focusOf({
      seen: inSight(),
      focused: document.hasFocus(),
      inMessenger: page.url.pathname.startsWith(BASE),
      active: chatStore.activeId,
    });
  const say = (chat: string | null) => {
    const at = Date.now();
    if (!focusToSay(chat, said, saidAt, at)) return;
    said = chat;
    saidAt = at;
    messengerApi.focus(chat).catch(() => {});
  };
  const look = () => say(now());
  const undoEffect = $effect.root(() => {
    // The chat, the page and the lock, as they change.
    $effect(() => {
      void lockSight.locked;
      look();
    });
  });
  window.addEventListener('focus', look);
  window.addEventListener('blur', look);
  document.addEventListener('visibilitychange', look);
  for (const t of TOUCHES) window.addEventListener(t, look, { capture: true, passive: true });
  return () => {
    undoEffect();
    window.removeEventListener('focus', look);
    window.removeEventListener('blur', look);
    document.removeEventListener('visibilitychange', look);
    for (const t of TOUCHES) window.removeEventListener(t, look, { capture: true });
    if (said !== null) messengerApi.focus(null).catch(() => {});
  };
}
