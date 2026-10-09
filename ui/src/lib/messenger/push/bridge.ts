// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the screens owe to notifications: a tap opens the chat it was about,
// and an opened chat takes its notification away.

import { goto } from '$app/navigation';
import { isTauriHost, messengerApi, PUSH_TAP_EVENT } from '../api';
import { BASE, chatHref, shouldFollow } from '../mobile/routes';
import { pushStore } from './pushStore.svelte';

/** What came from outside the app is checked before it is used as a route. */
const CHAT = /^(dm|group):[0-9a-f]{64}$/;

/**
 * A tap is taken from the phone once. Until the app has gone where it
 * leads, the route is also kept here: a page that reloads meanwhile (the
 * development server does that when the app comes back from the
 * background) goes on where it was going.
 */
const PENDING = 'veydan.push.route';

function keep(route: string) {
  try { sessionStorage.setItem(PENDING, route); } catch { /* no storage: the tap is followed once */ }
}

function kept(): string | null {
  try { return sessionStorage.getItem(PENDING); } catch { return null; }
}

function done() {
  try { sessionStorage.removeItem(PENDING); } catch { /* nothing kept */ }
}

async function takeRoute(): Promise<string | null> {
  const tap = await messengerApi.push.takeTap().catch(() => null);
  if (!tap) return kept();
  // No chat: the phone could not tell whose message it was; the list of
  // chats shows which one has something new.
  const route = tap.chat && CHAT.test(tap.chat) ? chatHref(tap.chat) : BASE;
  keep(route);
  return route;
}

/** Goes where a tap leads, and forgets it once there. */
export async function followRoute(route: string, replace = false) {
  try {
    await goto(route, { replaceState: replace });
  } finally {
    // A page that reloads meanwhile never gets here, and goes on later.
    done();
  }
}

async function followTap() {
  const route = await takeRoute();
  if (!route) return;
  // The chat of the tap is on the screen already: nothing to go to.
  if (!shouldFollow(route, location)) return done();
  await followRoute(route);
}

let launch: Promise<string | null> | null = null;
let launchClaimed = false;

/**
 * Where the tapped notification that started the app leads, if one did.
 *
 * The first to ask gets the route, everybody after gets null. The host's
 * start page asks it before sending the app to its default screen: the two
 * are one decision. Anybody else going to the chat on their own would race
 * the start page, which is shown only once the app lock lets it, and the
 * later of the two would win. The one who gets the route follows it with
 * `followRoute`.
 */
export async function launchRoute(): Promise<string | null> {
  launch ??= takeRoute();
  const route = await launch;
  if (!route || launchClaimed) return null;
  launchClaimed = true;
  return route;
}

let started = false;
let unlisten: (() => void) | null = null;

/** Started once, by the app's shell. */
export async function startPushBridge() {
  if (started) return;
  started = true;

  // Asked early, while the start page may still wait for the app lock.
  launch ??= takeRoute();

  // Taps while the app runs. Listened for before anything else can fail:
  // a page that missed this would miss every tap until it restarts.
  if (isTauriHost) {
    const { listen } = await import('@tauri-apps/api/event');
    unlisten = await listen(PUSH_TAP_EVENT, () => { followTap(); });
  }

  // At the start address the start page follows the tap; anywhere else (a
  // page reloaded while it was going to the chat) nobody else will.
  if (location.pathname !== '/') {
    const route = await launchRoute();
    if (route) await followRoute(route);
  }

  await pushStore.load();
}

// A module replaced during development takes its listener with it.
if (import.meta.hot) {
  import.meta.hot.dispose(() => { unlisten?.(); });
}

/**
 * The user is looking at this chat, or at the list of chats: what the
 * notifications said is on the screen now.
 *
 * A chat (`dm:<pubkey>`, `group:<id>`) takes away the notification of that
 * chat; the list of chats takes away the one about direct messages the
 * phone could not tell apart.
 */
export function pushSeen(chatId?: string) {
  if (!pushStore.view?.device.supported) return;
  const key = chatId && CHAT.test(chatId) ? chatId : 'dm';
  messengerApi.push.clear(key).catch(() => {});
}
