// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it, vi } from 'vitest';
import { LINK_EVENT, LinkRouter } from './router';

/** A shell whose event the test sends. */
function shell() {
  let handler: (() => void) | null = null;
  const listen = vi.fn(async (event: string, h: () => void) => {
    expect(event).toBe(LINK_EVENT);
    handler = h;
    return () => {
      handler = null;
    };
  });
  return { listen, send: () => handler?.(), listening: () => handler !== null };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('the router of links from the system', () => {
  it('asks the reader at the start: the app may have been started by a link', async () => {
    const s = shell();
    const router = new LinkRouter(s.listen);
    const reader = vi.fn(async () => true);
    router.provide(reader);
    await tick();
    expect(reader).toHaveBeenCalledTimes(1);
    expect(s.listening()).toBe(true);
  });

  it('asks again on every link the shell announces, and not without one', async () => {
    const s = shell();
    const router = new LinkRouter(s.listen);
    const reader = vi.fn(async () => true);
    router.provide(reader);
    await tick();
    await router.pull();
    expect(reader).toHaveBeenCalledTimes(1);
    s.send();
    await tick();
    expect(reader).toHaveBeenCalledTimes(2);
  });

  it('leaves the link waiting while the reader cannot read, and reads it when it can', async () => {
    const s = shell();
    const router = new LinkRouter(s.listen);
    let ready = false;
    const reader = vi.fn(async () => ready);
    router.provide(reader);
    await tick();
    expect(reader).toHaveBeenCalledTimes(1);
    // Locked: the next pull asks again, the link is still there.
    ready = true;
    await router.pull();
    expect(reader).toHaveBeenCalledTimes(2);
    // Read: nothing waits any more.
    await router.pull();
    expect(reader).toHaveBeenCalledTimes(2);
  });

  it('reads a link that came while another was read', async () => {
    const s = shell();
    const router = new LinkRouter(s.listen);
    let release: () => void = () => {};
    const reader = vi.fn(() => new Promise<boolean>((r) => { release = () => r(true); }));
    router.provide(reader);
    await tick();
    s.send();
    release();
    await tick();
    expect(reader).toHaveBeenCalledTimes(2);
    release();
    await tick();
    expect(reader).toHaveBeenCalledTimes(2);
  });

  it('keeps a link that came before any reader for the first one', async () => {
    const s = shell();
    const router = new LinkRouter(s.listen);
    await router.arrived();
    const reader = vi.fn(async () => true);
    router.provide(reader);
    await tick();
    expect(reader).toHaveBeenCalledTimes(1);
  });

  it('a reader that failed leaves the link for the next one', async () => {
    const router = new LinkRouter(null);
    const stop = router.provide(async () => { throw new Error('locked'); });
    await tick();
    stop();
    const next = vi.fn(async () => true);
    router.provide(next);
    await tick();
    expect(next).toHaveBeenCalledTimes(1);
  });
});
