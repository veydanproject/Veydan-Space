// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Autosave of the phone's note editor, without the DOM. One write at a time;
// every edit bumps a counter, and a write covers only the edits made before it
// started. An edit typed while a write is in flight keeps the buffer unsaved,
// so it is written next instead of being hidden behind a "saved" badge.

export type SaveStatus = 'idle' | 'dirty' | 'saving' | 'saved' | 'failed';

export interface SaveHooks {
  /** Writes the buffer as it is at the call. False when there was nothing to write (an empty new note). */
  write(): Promise<boolean>;
  /** Writes are held: an external change waits for the user. */
  held(): boolean;
  /** A write threw. True when it is a conflict that now holds writes, false for a plain failure. */
  failed(e: unknown): boolean;
  /** The badge state. */
  status(s: SaveStatus): void;
  /** After every write, whatever its end. */
  settled?(): void;
}

export class SaveQueue {
  /** Edits so far. */
  private rev = 0;
  /** Edits the disk holds. */
  private savedRev = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private flight: Promise<boolean> | null = null;

  constructor(
    private readonly hooks: SaveHooks,
    private readonly delay = 700,
  ) {}

  /** The buffer holds edits the disk does not, whatever the badge says. */
  get unsaved(): boolean {
    return this.rev !== this.savedRev;
  }

  /** A write is in flight. */
  get busy(): boolean {
    return this.flight !== null;
  }

  /** An edit of the buffer: written after a pause, unless writes are held. */
  edit(): void {
    this.rev++;
    this.hooks.status('dirty');
    clearTimeout(this.timer);
    if (this.hooks.held()) return;
    this.timer = setTimeout(() => void this.flush(false), this.delay);
  }

  /** The buffer was replaced by the disk's text (loaded, restored, discarded): nothing to write. */
  discard(): void {
    clearTimeout(this.timer);
    this.savedRev = this.rev;
  }

  /** Writes every pending edit, the ones typed during the write too; resolves when the disk holds them or a write failed. */
  save(): Promise<void> {
    return this.flush(true);
  }

  private async flush(all: boolean): Promise<void> {
    clearTimeout(this.timer);
    for (;;) {
      if (this.hooks.held()) return;
      if (this.flight) {
        await this.flight;
        continue;
      }
      if (!this.unsaved) return;
      this.flight = this.run(this.rev);
      const ok = await this.flight;
      // An edit made meanwhile has its own timer; an explicit save does not wait for it.
      if (!ok || !all) return;
    }
  }

  private async run(rev: number): Promise<boolean> {
    this.hooks.status('saving');
    try {
      const wrote = await this.hooks.write();
      this.savedRev = Math.max(this.savedRev, rev);
      this.hooks.status(this.unsaved ? 'dirty' : wrote ? 'saved' : 'idle');
      return true;
    } catch (e) {
      const conflict = this.hooks.failed(e);
      this.hooks.status(conflict ? 'dirty' : 'failed');
      return false;
    } finally {
      this.flight = null;
      this.hooks.settled?.();
    }
  }
}
