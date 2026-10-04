// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the links met in messages lead to. Links inside are asked of the
// runtime, many at once, and asked again when a group or a person
// changes. Previews of pages outside are asked one by one, and only when
// the user said so.

import { messengerApi, previewErrorCode, type LinkPreview, type LinkView, type MessengerUiEvent } from '../api';
import type { ExternalUrl, InternalLinkText } from './types';

/** As many as the runtime answers in one call. */
const BATCH = 64;

export type PreviewState =
  | { status: 'loading' }
  | { status: 'ready'; preview: LinkPreview }
  | { status: 'failed'; code: string };

class LinkStore {
  private views = $state<Record<string, LinkView>>({});
  private previews = $state<Record<string, PreviewState>>({});
  private waiting = new Set<InternalLinkText>();
  private asked = new Set<string>();
  private _timer: ReturnType<typeof setTimeout> | null = null;
  private _again: ReturnType<typeof setTimeout> | null = null;

  /** What the link leads to; `null` until known. Reading it asks once. */
  view(link: InternalLinkText): LinkView | null {
    const known = this.views[link];
    if (known === undefined) this.ask(link);
    return known ?? null;
  }

  /** The same, for whoever cannot wait for a redraw. */
  async resolve(link: InternalLinkText): Promise<LinkView> {
    const [view] = await messengerApi.links.inspect([link]);
    const got = view ?? { kind: 'invalid', code: 'link_bad_scheme' };
    this.asked.add(link);
    this.views = { ...this.views, [link]: got };
    return got;
  }

  private ask(link: InternalLinkText) {
    if (this.asked.has(link)) return;
    this.asked.add(link);
    this.waiting.add(link);
    if (this._timer) return;
    // Outside the render that asked: state must not change while reading it.
    this._timer = setTimeout(() => {
      this._timer = null;
      this.flush().catch(() => {});
    }, 0);
  }

  private async flush() {
    const all = [...this.waiting];
    this.waiting.clear();
    for (let i = 0; i < all.length; i += BATCH) {
      const part = all.slice(i, i + BATCH);
      try {
        const got = await messengerApi.links.inspect(part);
        const next = { ...this.views };
        part.forEach((link, n) => { if (got[n]) next[link] = got[n]; });
        this.views = next;
      } catch {
        // Not answered (locked, no session): asked again when next read.
        for (const link of part) this.asked.delete(link);
      }
    }
  }

  /** Something changed: what is shown is asked again, the cards stay while it is. */
  refresh() {
    if (this._again) return;
    this._again = setTimeout(() => {
      this._again = null;
      for (const link of Object.keys(this.views)) this.waiting.add(link as InternalLinkText);
      this.flush().catch(() => {});
    }, 250);
  }

  preview(url: ExternalUrl): PreviewState | null {
    return this.previews[url] ?? null;
  }

  async askPreview(url: ExternalUrl) {
    if (this.previews[url]?.status === 'loading') return;
    this.previews = { ...this.previews, [url]: { status: 'loading' } };
    try {
      const preview = await messengerApi.links.preview(url);
      this.previews = { ...this.previews, [url]: { status: 'ready', preview } };
    } catch (e) {
      this.previews = { ...this.previews, [url]: { status: 'failed', code: previewErrorCode(e) ?? 'preview_failed' } };
    }
  }

  hidePreview(url: ExternalUrl) {
    const { [url]: _gone, ...rest } = this.previews;
    this.previews = rest;
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    switch (ev.name) {
      case 'group.updated':
      case 'profile.updated':
      case 'follows.updated':
      case 'dm.relationship':
      case 'chats.updated':
        if (Object.keys(this.views).length) this.refresh();
        break;
    }
  }

  reset() {
    this.views = {};
    this.previews = {};
    this.waiting.clear();
    this.asked.clear();
  }
}

export const linkStore = new LinkStore();
