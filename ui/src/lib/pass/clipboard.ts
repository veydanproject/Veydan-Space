/** Clipboard helper for secrets: the value leaves the clipboard again unless something else was copied since. */

export interface CopySecretOptions {
  /** Milliseconds until the clipboard is cleared; 0 keeps the value. Default 30 s. */
  clearAfterMs?: number;
  /** Writer tried when `navigator.clipboard` refuses (the backend clipboard on desktop). */
  fallback?: (value: string) => Promise<void>;
}

function legacyCopy(value: string): void {
  if (typeof document === 'undefined') throw new Error('copy');
  const el = document.createElement('textarea');
  el.value = value;
  el.setAttribute('readonly', '');
  el.style.position = 'fixed';
  el.style.left = '-9999px';
  document.body.appendChild(el);
  el.select();
  const ok = document.execCommand('copy');
  el.remove();
  if (!ok) throw new Error('copy');
}

/** Writes `value` to the clipboard and, after `clearAfterMs`, empties it only if it still holds `value`. */
export async function copySecret(value: string, options: CopySecretOptions = {}): Promise<void> {
  const { clearAfterMs = 30_000, fallback } = options;
  try {
    await navigator.clipboard.writeText(value);
  } catch {
    if (fallback) await fallback(value);
    else legacyCopy(value);
  }
  if (clearAfterMs <= 0) return;
  setTimeout(() => {
    navigator.clipboard
      .readText()
      .then((current) => {
        if (current === value) return navigator.clipboard.writeText('');
      })
      .catch(() => {});
  }, clearAfterMs);
}
