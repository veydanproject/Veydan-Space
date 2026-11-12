// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The unsent text of each conversation: the composer parks it when the
// conversation changes, the chat list shows «Черновик:». The runtime keeps
// it on this device (`msg_drafts`), so it outlives the app (ROO-1112); it
// is never sent anywhere and goes with the account.
//
// A change is written within `SAVE_MS`, at once when the page is hidden or
// left (the phone's Home, a closed window) and at once when the field is
// sent or emptied: a sent text must not come back after a kill.

import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { messengerApi } from '../api';

export const drafts: SvelteMap<string, string> = ((globalThis as Record<string, unknown>).__msgDrafts ??= new SvelteMap()) as SvelteMap<string, string>;

/** The conversations whose draft is written in Markdown (the composer's format menu). */
export const markdownDrafts: Set<string> = ((globalThis as Record<string, unknown>).__msgMarkdownDrafts ??= new Set()) as Set<string>;

/** Raised when the drafts kept by the runtime came in: an open composer takes its own. */
export const draftsLoaded = $state({ n: 0 });

const SAVE_MS = 400;
const owed = new Set<string>();
let timer: ReturnType<typeof setTimeout> | null = null;
/** Raised by every load and by a reset: an answer to an older load is dropped. */
let generation = 0;
/** Changed here (emptied too) since the last load began: what it brings for them is older. */
const touched = new Set<string>();

function write(key: string) {
  messengerApi.drafts.set(key, drafts.get(key) ?? '', markdownDrafts.has(key)).catch(() => {
    // No runtime (the browser, the module off): kept in memory only.
  });
}

/** Write every draft owed now. */
export function flushDrafts() {
  if (timer) clearTimeout(timer);
  timer = null;
  for (const key of owed) write(key);
  owed.clear();
}

/**
 * The field of `key` now reads `text`. Nothing is written when nothing
 * changed (a composer that opens empty before the drafts came in must not
 * forget the kept one). It reads the drafts untracked: the composer's
 * effect that calls it must not wake when they come in, or it would empty
 * the one just loaded before the field takes it.
 */
export function setDraft(key: string, text: string, md: boolean) {
  untrack(() => put(key, text, md));
}

function put(key: string, text: string, md: boolean) {
  const value = text.trim() ? text : '';
  const asMd = !!value && md;
  if ((drafts.get(key) ?? '') === value && markdownDrafts.has(key) === asMd) return;
  touched.add(key);
  if (value) drafts.set(key, value); else drafts.delete(key);
  if (asMd) markdownDrafts.add(key); else markdownDrafts.delete(key);
  if (!value) {
    owed.delete(key);
    write(key);
    return;
  }
  owed.add(key);
  timer ??= setTimeout(flushDrafts, SAVE_MS);
}

/**
 * The drafts the runtime keeps; one typed, sent or emptied here meanwhile
 * stands, and nothing comes in after a reset.
 */
export async function loadDrafts() {
  const g = ++generation;
  touched.clear();
  const kept = await messengerApi.drafts.get();
  if (g !== generation) return;
  for (const d of kept) {
    if (drafts.has(d.chat) || touched.has(d.chat) || !d.text.trim()) continue;
    drafts.set(d.chat, d.text);
    if (d.md) markdownDrafts.add(d.chat);
  }
  draftsLoaded.n++;
}

/** The module stopped or the identity went: what it showed goes too. */
export function resetDrafts() {
  if (timer) clearTimeout(timer);
  timer = null;
  owed.clear();
  generation++;
  touched.clear();
  drafts.clear();
  markdownDrafts.clear();
}

if (typeof window !== 'undefined' && !(globalThis as Record<string, unknown>).__msgDraftsFlush) {
  (globalThis as Record<string, unknown>).__msgDraftsFlush = true;
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushDrafts(); });
  window.addEventListener('pagehide', flushDrafts);
  window.addEventListener('blur', flushDrafts);
}
