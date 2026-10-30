// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The system «Назад» closes the sheet: the sheet takes one place in the
// history while it is open, so a back pops that place (and closes the
// sheet) instead of leaving the chat. Closing the sheet another way gives
// the place back. Places held at once (the mode of choosing messages, the
// sheet of a forward over it) are a stack: a back pops only the newest,
// and places let go are given back from the top down one at a time,
// whatever the order they were let go in (two backs asked together would
// go back once).

import { pushState } from '$app/navigation';
import { page } from '$app/state';
import { BACK_WAIT_MS } from '$lib/core/ui/back';

/** The browser, given to a test in its place. */
export interface SheetBackEnv {
  /** Takes a place in the history. */
  push(): void;
  /** Whether that place is the current one (a navigation of the page may have taken it already). */
  stepped(): boolean;
  back(): void;
  events: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>;
}

/** What holds a place of the history (`App.PageState`). */
export type BackPlace = 'attachSheet' | 'chooseMessages' | 'forwardSheet';

const browser = (place: BackPlace): SheetBackEnv => ({
  push: () => pushState('', { ...page.state, [place]: true }),
  stepped: () => !!page.state[place],
  back: () => history.back(),
  events: window,
});

/** A place held: who holds it and whether it was let go. */
interface Held {
  env: SheetBackEnv;
  released: boolean;
}

/** The places held, the newest last. */
let held: Held[] = [];
/** A back of ours is on its way. */
let going = false;

const drop = (h: Held) => {
  const i = held.indexOf(h);
  if (i >= 0) held.splice(i, 1);
};

/** Gives back the places let go from the top down: one let go under one still
 * held waits for it, whatever the order the two were let go in. */
function giveBack() {
  if (going) return;
  const top = held[held.length - 1];
  if (!top?.released) return;
  // The page may have left the place meanwhile (a back of its own, a navigation).
  if (!top.env.stepped()) {
    drop(top);
    return giveBack();
  }
  going = true;
  const arrived = () => {
    clearTimeout(timer);
    top.env.events.removeEventListener('popstate', arrived);
    going = false;
    drop(top);
    giveBack();
  };
  const timer = setTimeout(arrived, BACK_WAIT_MS);
  top.env.events.addEventListener('popstate', arrived);
  top.env.back();
}

/** Call while the sheet is open; `close` runs on a back. Returns the way to let go. */
export function holdBack(close: () => void, env: SheetBackEnv | BackPlace = 'attachSheet'): () => void {
  const e = typeof env === 'string' ? browser(env) : env;
  const h: Held = { env: e, released: false };
  const onpop = () => {
    // A newer place was popped: this one is the current one again.
    if (e.stepped()) return;
    e.events.removeEventListener('popstate', onpop);
    drop(h);
    close();
    // A place let go under this one owes its back now.
    giveBack();
  };
  e.push();
  held.push(h);
  e.events.addEventListener('popstate', onpop);
  return () => {
    e.events.removeEventListener('popstate', onpop);
    if (h.released || !held.includes(h)) return;
    h.released = true;
    giveBack();
  };
}

/** Only for tests: forget the backs under way. */
export function resetBacks() {
  held = [];
  going = false;
}
