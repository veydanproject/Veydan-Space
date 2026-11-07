// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The room a fixed stack takes, told to the page as a CSS variable on <html>:
 * its height and a gap of --sp-2 while it holds anything, unset while empty.
 * The toast stands by it (ToastHost): over the cloud's banners on a computer
 * (`--zone-cloud`), under the cards of new messages on a phone (`--zone-notice`),
 * never on them.
 */
export function zone(node: HTMLElement, name: string | null) {
  let current = name;
  const root = document.documentElement;
  const tell = () => {
    if (!current) return;
    const h = node.offsetHeight;
    if (h > 0) root.style.setProperty(current, `calc(${h}px + var(--sp-2))`);
    else root.style.removeProperty(current);
  };
  const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(tell);
  ro?.observe(node);
  tell();
  return {
    update(next: string | null) {
      if (current && current !== next) root.style.removeProperty(current);
      current = next;
      tell();
    },
    destroy() {
      ro?.disconnect();
      if (current) root.style.removeProperty(current);
    },
  };
}
