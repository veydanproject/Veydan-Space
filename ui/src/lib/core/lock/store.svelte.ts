// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/core/api';
import type { LockSecret, LockStatus } from '$lib/core/types';

const TOUCH_INTERVAL_MS = 20_000;

/** Lock state of the app, shared by every surface; the backend is the source of truth. */
class LockStore {
  status = $state<LockStatus>({
    enabled: false,
    locked: false,
    timeout_min: 5,
    vault: 'none',
    kind: 'password',
    hint: null,
    has_recovery: false,
  });
  ready = $state(false);
  /** The lock screen shows the recovery flow (steps 1–3, AppLockGate). */
  recovering = $state(false);
  /**
   * The recovery key a recovery just issued, until the user confirms step 3.
   * The recovery unlocked the app and the old key no longer works: this one is
   * shown once, so it outlives a lock that comes before Done (it is shown
   * again after the unlock) and is kept in memory only.
   */
  newRecoveryKey = $state<string | null>(null);

  private _lastTouch = 0;
  private _unlisten: (() => void) | null = null;
  /** Bumped on unlock/lock so an older status read cannot overwrite it. */
  private _epoch = 0;

  get locked() {
    return this.status.enabled && this.status.locked;
  }

  /**
   * Whether the app is behind the lock gate: locked, or in the recovery flow
   * (which unlocks the app before its step 3). The shells show nothing of the
   * app — title-bar tools, Settings, overlays — while it is.
   */
  get gated() {
    return this.locked || this.recovering || this.newRecoveryKey !== null;
  }

  async refresh() {
    const epoch = this._epoch;
    try {
      const status = await api.lock.status();
      if (epoch !== this._epoch) return;
      this.status = status;
    } catch {}
    this.ready = true;
  }

  async unlock(password: string) {
    const epoch = ++this._epoch;
    const status = await api.lock.unlock(password);
    if (epoch !== this._epoch) return;
    this.status = status;
  }

  async lock() {
    const epoch = ++this._epoch;
    const status = await api.lock.lock();
    if (epoch !== this._epoch) return;
    this.status = status;
  }

  /** Enable, change or remove the lock. Returns the recovery key when one was just created. */
  async setSecret(secret: LockSecret | null, current?: string): Promise<string | null> {
    const { recovery_key, ...status } = await api.lock.set(secret, current);
    this.status = status;
    return recovery_key;
  }

  /** New recovery key; the previous one stops working. */
  async regenerateRecovery(current: string): Promise<string> {
    const code = await api.lock.recoveryRegenerate(current);
    await this.refresh();
    return code;
  }

  /** Replace the lock secret with the recovery key; returns the new recovery key. */
  async recover(code: string, secret: LockSecret): Promise<string> {
    const epoch = ++this._epoch;
    const { recovery_key, ...status } = await api.lock.recover(code, secret);
    if (epoch === this._epoch) this.status = status;
    this.newRecoveryKey = recovery_key ?? '';
    return this.newRecoveryKey;
  }

  /** Step 3 of the recovery was confirmed: the app opens. */
  finishRecovery() {
    this.newRecoveryKey = null;
    this.recovering = false;
  }

  async setTimeout(minutes: number) {
    this.status = await api.lock.timeoutSet(minutes);
  }

  /** Throttled activity ping so the inactivity timer restarts. */
  touch() {
    if (!this.status.enabled || this.status.locked) return;
    const now = Date.now();
    if (now - this._lastTouch < TOUCH_INTERVAL_MS) return;
    this._lastTouch = now;
    void api.lock.touch();
  }

  /** Subscribe to lock events and user activity from any window; idempotent. */
  async listen() {
    if (this._unlisten || typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    const touch = () => this.touch();
    window.addEventListener('keydown', touch, true);
    window.addEventListener('pointerdown', touch, true);
    this._unlisten = () => {
      window.removeEventListener('keydown', touch, true);
      window.removeEventListener('pointerdown', touch, true);
    };
    const { listen } = await import('@tauri-apps/api/event');
    const a = await listen('lock://locked', () => void this.refresh());
    const b = await listen('lock://unlocked', () => void this.refresh());
    const c = await listen('passwords://vault-changed', () => void this.refresh());
    const dom = this._unlisten;
    this._unlisten = () => { dom(); a(); b(); c(); };
  }
}

export const appLock = new LockStore();
