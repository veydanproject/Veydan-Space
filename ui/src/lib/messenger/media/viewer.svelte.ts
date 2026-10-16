// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Full-screen viewer state: one picture or video at a time, paged through
// the pictures and videos of its album.
//
// Opening the viewer is a step of history: the phone's way back (the gesture,
// the button) closes it instead of leaving the chat under it.

import { SvelteMap } from 'svelte/reactivity';
import { pushState } from '$app/navigation';
import { page } from '$app/state';

export interface ViewerItem {
  messageId: string;
  kind: 'image' | 'video';
  src: string;
  name: string;
}

class Viewer {
  /** Pictures and videos on this device, by message: a part of an album is paged to once it is here. */
  readonly here = new SvelteMap<string, ViewerItem>();
  item = $state<ViewerItem | null>(null);
  /** The album the shown one belongs to, in its order. */
  private album = $state<string[]>([]);
  /** The step of history opening made, while it is the current one. */
  private stepped = false;

  /** What can be paged through: the parts of the album that are here. */
  readonly items = $derived(this.album.map((id) => this.here.get(id)).filter((it): it is ViewerItem => !!it));
  readonly index = $derived(this.item ? this.items.findIndex((it) => it.messageId === this.item?.messageId) : -1);

  /** Shows `item`; `album` is the ids of its album's parts, in order (itself alone when none). */
  open(item: ViewerItem, album: string[] = [item.messageId]) {
    if (!this.item) {
      try { pushState('', { ...page.state, mediaViewer: true }); this.stepped = true; } catch { this.stepped = false; }
    }
    this.album = album.includes(item.messageId) ? album : [item.messageId];
    this.item = item;
  }

  /** The next (1) or the previous (-1) part of the album; nothing past its ends. */
  step(by: 1 | -1) {
    const next = this.items[this.index + by];
    if (this.index >= 0 && next) this.item = next;
  }

  /** The user closes it: back over the step it made, which closes it (`left`). */
  dismiss() {
    if (this.stepped && page.state.mediaViewer) history.back();
    else this.close();
  }

  /** History moved off the viewer's step (a back): it closes. */
  left() {
    if (this.item && this.stepped && !page.state.mediaViewer) this.close();
  }

  /** Closes it at once; a step it made stays in history, an ordinary page of the chat. */
  close() {
    this.item = null;
    this.album = [];
    this.stepped = false;
  }
}

export const viewer = new Viewer();
