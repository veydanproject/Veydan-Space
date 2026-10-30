// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The one way in for the links the system hands to the app: a `veydan://…`
// clicked in a browser, a mail or another app (crates/shell/src/links.rs).
// The shell keeps the last link in memory and says `links://incoming`,
// without the link. The module that shows links (the messenger: a link is
// its card) gives a reader here; the router asks it to read whenever a link
// may wait — at the start (the link the app was started with), on every
// event, and when the module says it can read again (after the app is
// unlocked). A reader that cannot read yet leaves the link with the shell.
// Nothing is opened here: the reader shows a card, and the card acts only
// when the user presses its button.

export const LINK_EVENT = 'links://incoming';

/** Takes the waiting link and shows it; false when it cannot read now (locked, not started). */
export type LinkReader = () => Promise<boolean>;

type Listen = (event: string, handler: () => void) => Promise<() => void>;

export class LinkRouter {
  private reader: LinkReader | null = null;
  /** A link may wait with the shell: at the start, the one the app was started with. */
  private waiting = true;
  private reading: Promise<void> | null = null;
  private listening: Promise<void> | null = null;
  private unlisten: (() => void) | null = null;

  constructor(private readonly listen: Listen | null) {}

  /** The module that shows links; what it returns takes it back. */
  provide(reader: LinkReader): () => void {
    this.reader = reader;
    void this.start().then(() => this.pull());
    return () => {
      if (this.reader === reader) this.reader = null;
    };
  }

  /** The reader can read now, or a link came: asks it, as long as one may wait. */
  pull(): Promise<void> {
    this.reading ??= this.drain().finally(() => {
      this.reading = null;
    });
    return this.reading;
  }

  /** The shell said a link came. */
  arrived(): Promise<void> {
    this.waiting = true;
    return this.pull();
  }

  stop() {
    this.unlisten?.();
    this.unlisten = null;
    this.listening = null;
  }

  private start(): Promise<void> {
    this.listening ??= (this.listen?.(LINK_EVENT, () => void this.arrived()) ?? Promise.resolve(() => {}))
      .then((stop) => {
        this.unlisten = stop;
      })
      // Without the event the start and the reader's own pulls still read.
      .catch(() => {});
    return this.listening;
  }

  private async drain() {
    // A link that comes while one is read is read after it.
    while (this.waiting && this.reader) {
      this.waiting = false;
      const read = await this.reader().catch(() => false);
      if (!read) {
        this.waiting = true;
        return;
      }
    }
  }
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function tauriListen(event: string, handler: () => void) {
  const { listen } = await import('@tauri-apps/api/event');
  return listen(event, handler);
}

export const linkRouter = new LinkRouter(isTauri ? tauriListen : null);

// A module replaced during development takes its listener with it.
if (import.meta.hot) {
  import.meta.hot.dispose(() => linkRouter.stop());
}
