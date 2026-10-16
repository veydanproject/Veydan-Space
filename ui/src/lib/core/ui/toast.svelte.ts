// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The one toast of every app (design kit, FEEDBACK_STANDARD §1–§3, §5): one
// at a time, and how it looks and how long it stays depend on its type alone.
//
//   toast.success('Текст скопирован')                          T1 · 2 s
//   toast.secret({ clear })                                    T2 · 30 s · «Буфер очистится через N с» · «Очистить сейчас»
//   toast.undo('Чат перенесён в архив', { onAction: unarchive }) T3 · 8 s · «Отменить» · countdown bar
//   toast.error('Не удалось закрепить чат', { onAction: retry }) T5 · until closed · «Повторить» · ×
//   toast.info('…')                                            T11 · 4 s
//
// A new toast replaces the shown one; an error is replaced only by another
// error or by the result of its own retry (`retryOf`), anything else waits
// (the newest one) until the error goes. Hovering or focusing the toast
// pauses its countdown. Shown by core/ui/ToastHost.svelte, in each shell.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';

export type ToastType = 'success' | 'secret' | 'undo' | 'error' | 'info';

export interface ToastOptions {
  /** Second line (T3: what follows from the action). T2 has its own countdown line. */
  sub?: string;
  /** Override of the type's icon (T3: the icon of the action — trash, archive…). */
  icon?: string;
  /** The one action's label; the type's default otherwise («Отменить», «Повторить», «Очистить сейчас»), `false` for none. */
  action?: string | false;
  onAction?: () => void;
  /** Milliseconds; 0 stays until closed. */
  duration?: number;
  /** The error toast this one answers (the result of its retry may replace it). */
  retryOf?: number;
}

export interface Toast extends ToastOptions {
  id: number;
  type: ToastType;
  title: string;
  /** Milliseconds left; counts down while not paused. */
  left: number;
  total: number;
}

const DURATION: Record<ToastType, number> = { success: 2000, secret: 30000, undo: 8000, error: 0, info: 4000 };

let seq = 0;
let timer: ReturnType<typeof setInterval> | null = null;
let last = 0;

class ToastStore {
  current = $state<Toast | null>(null);
  pending: Toast | null = null;
  paused = $state(false);

  show(type: ToastType, title: string, opts: ToastOptions = {}): number {
    const total = opts.duration ?? DURATION[type];
    const next: Toast = { ...opts, id: ++seq, type, title, left: total, total };
    const cur = this.current;
    if (cur?.type === 'error' && type !== 'error' && opts.retryOf !== cur.id) {
      this.pending = next;
      return next.id;
    }
    this.current = next;
    this.paused = false;
    this.tick();
    return next.id;
  }

  success(title: string, opts?: ToastOptions) { return this.show('success', title, opts); }
  secret(title: string, opts?: ToastOptions) { return this.show('secret', title, opts); }
  undo(title: string, opts?: ToastOptions) { return this.show('undo', title, opts); }
  error(title: string, opts?: ToastOptions) { return this.show('error', title, opts); }
  info(title: string, opts?: ToastOptions) { return this.show('info', title, opts); }

  close(id?: number) {
    if (!this.current || (id !== undefined && this.current.id !== id)) return;
    this.current = this.pending;
    this.pending = null;
    this.paused = false;
    this.tick();
  }

  act() {
    const cur = this.current;
    if (!cur) return;
    this.close(cur.id);
    cur.onAction?.();
  }

  private tick() {
    if (timer) clearInterval(timer);
    timer = null;
    const cur = this.current;
    if (!cur || cur.total <= 0) return;
    last = Date.now();
    timer = setInterval(() => {
      const now = Date.now();
      const c = this.current;
      if (!c) return;
      if (!this.paused) c.left = Math.max(0, c.left - (now - last));
      last = now;
      if (c.left <= 0) this.close(c.id);
    }, 250);
  }
}

export const toast = new ToastStore();

/** Something was copied: T1 «Скопировано» (or the given title). */
export function copied(title?: string) {
  return toast.success(title ?? (get(t) as (k: string) => string)('toast_copied'));
}

/** Empties the clipboard: «Очистить сейчас» of a copied secret (the backend's clipboard first, then the page's). */
export async function clearClipboard(): Promise<void> {
  try {
    const { api } = await import('$lib/core/api');
    await api.system.clipboardWriteText('');
  } catch {
    await navigator.clipboard?.writeText('').catch(() => {});
  }
}

/** A secret is in the clipboard (it leaves it by itself in 30 s): T2 with «Очистить сейчас». */
export function secretCopied(title?: string) {
  return toast.secret(title ?? (get(t) as (k: string) => string)('toast_copied'), { onAction: () => void clearClipboard() });
}
