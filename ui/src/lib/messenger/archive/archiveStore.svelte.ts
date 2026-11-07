// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The archive of chats as the screens see it (messenger-archive-plan.md
// section 5): one state read by the blocking screen after a restore (step B),
// the banner above the chats (step C), the row in an open chat and the card
// in the settings. Asked at start (`messenger_archive_status`), then moved by
// the events `archive.progress` and `archive.synced`; a change of phase or of
// the month under way asks the status again (at most once in a few seconds):
// the size, the months and the scopes still to come are in it alone.

import { messengerApi, type MessengerUiEvent } from '../api';
import { ARCHIVE_GATE_EVENT, ARCHIVE_GATE_KEY } from '$lib/core/ui/archiveGate';
import { etaSeconds, GATE_BUDGET_MS, percentOf } from './archive';
import { IDLE_PROGRESS, type ArchiveProgress, type ArchiveStatus } from './types';

/** The status is asked again no more often than this on a change of phase or month. */
export const RELOAD_EVERY_MS = 5_000;

function readGate(): number {
  try {
    return Number(sessionStorage.getItem(ARCHIVE_GATE_KEY)) || 0;
  } catch {
    return 0;
  }
}

class ArchiveStore {
  status = $state<ArchiveStatus | null>(null);
  /** Step B is held: a restore or a linking just ended and the first 20 s are not over. */
  armed = $state(false);
  /** The rate of bytes is measured from the first sample of a restore. */
  private since: { at: number; bytes: number } | null = null;
  private now = $state(Date.now());
  private tick: ReturnType<typeof setInterval> | null = null;
  private release: ReturnType<typeof setTimeout> | null = null;
  /** When the status was last asked, and whether a reload waits for the throttle. */
  private loadedAt = 0;
  private reload: ReturnType<typeof setTimeout> | null = null;

  progress = $derived<ArchiveProgress>(this.status?.progress ?? IDLE_PROGRESS);
  available = $derived(this.status?.available ?? false);
  restoring = $derived(this.available && this.progress.phase === 'restoring' && this.progress.total > 0);
  percent = $derived(percentOf(this.progress));
  /** Step B shows: held, a restore under way, and something to restore. */
  blocking = $derived(this.armed && this.restoring);
  /** Seconds left of the restore by the rate of bytes, null while it is not known. */
  eta = $derived(etaSeconds(this.progress, this.since, this.now));

  constructor() {
    // A page that came back (a linking reloads it) finds step B still held.
    this.catchUp();
    if (typeof window !== 'undefined') window.addEventListener(ARCHIVE_GATE_EVENT, () => this.catchUp());
  }

  /** Whether the segments of a scope (`dm`, `dm@<server>`, `group:<id>`) are still to come. */
  isRestoring(scope: string | null): boolean {
    return this.restoring && scope !== null && (this.status?.restoring_scopes ?? []).includes(scope);
  }

  /** Takes the hold the cloud pages wrote (core/ui/archiveGate.ts), for what is left of the budget. */
  private catchUp() {
    const at = typeof sessionStorage === 'undefined' ? 0 : readGate();
    const left = at ? at + GATE_BUDGET_MS - Date.now() : 0;
    if (left > 0) this.hold(left);
  }

  private hold(ms: number) {
    this.armed = true;
    if (this.release) clearTimeout(this.release);
    // After the budget the app opens and step C takes over; the restore goes on behind.
    this.release = setTimeout(() => this.open(), ms);
  }

  /** Step B is over: the budget passed, or the restore did. */
  open() {
    this.armed = false;
    if (this.release) clearTimeout(this.release);
    this.release = null;
    try {
      sessionStorage.removeItem(ARCHIVE_GATE_KEY);
    } catch { /* nothing to remove */ }
  }

  async load() {
    this.loadedAt = Date.now();
    const status = await messengerApi.archive.status();
    this.apply(status);
  }

  /** Ask the status again, at most once in `RELOAD_EVERY_MS`; a reload already waiting is kept. */
  private reloadSoon() {
    if (this.reload) return;
    const wait = Math.max(0, this.loadedAt + RELOAD_EVERY_MS - Date.now());
    this.reload = setTimeout(() => {
      this.reload = null;
      this.load().catch(() => {});
    }, wait);
  }

  private apply(status: ArchiveStatus | null) {
    this.status = status;
    // Nothing to restore on a device with no storage: the step is skipped. An idle archive with no total yet may still start (the budget ends the hold).
    if (status && !status.available) this.open();
    this.measure(status?.progress ?? IDLE_PROGRESS);
  }

  /** Starts the clock of the estimate with a restore and stops it with the end of one. */
  private measure(p: ArchiveProgress) {
    if (p.phase !== 'restoring') {
      this.since = null;
      if (this.tick) clearInterval(this.tick);
      this.tick = null;
      return;
    }
    this.since ??= { at: Date.now(), bytes: p.bytes_done };
    this.tick ??= setInterval(() => { this.now = Date.now(); }, 1000);
  }

  handleEvent(event: MessengerUiEvent) {
    if (event.name === 'archive.progress') {
      const progress = event.payload as ArchiveProgress;
      const before = this.status?.progress;
      const base: ArchiveStatus = this.status ?? { available: true, progress, size_bytes: 0, from_month: null, to_month: null, paused: null, restoring_scopes: [], too_large: [], not_backed_up: [] };
      // The progress says nothing of the storage: a device told it has none keeps that word until the status says otherwise.
      this.status = { ...base, progress, paused: progress.phase === 'paused' ? (progress.reason ?? base.paused) : null };
      this.measure(progress);
      if (!before || before.phase !== progress.phase || before.month !== progress.month) this.reloadSoon();
    } else if (event.name === 'archive.synced') {
      // Every segment is back: the banner and the row go, the card learns the size and the period.
      if (this.status) this.status = { ...this.status, progress: { ...IDLE_PROGRESS }, restoring_scopes: [] };
      this.measure(IDLE_PROGRESS);
      this.open();
      this.load().catch(() => {});
    }
  }

  reset() {
    this.status = null;
    this.since = null;
    this.measure(IDLE_PROGRESS);
    if (this.reload) clearTimeout(this.reload);
    this.reload = null;
    this.open();
  }
}

export const archiveStore = new ArchiveStore();
