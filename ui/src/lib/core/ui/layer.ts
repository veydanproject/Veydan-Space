// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * A modal layer (a drawer, a dialog, a confirmation) holds the keyboard: `use:layer`
 * after `use:portal` on its backdrop. While it is open everything else in the body is
 * inert, focus moves into it, Tab and Shift+Tab go round inside it, and when it closes
 * focus goes back where it came from. Layers stack: the top one holds, the one under it
 * is inert with the rest until the top closes. What the body gets later (a menu or a
 * select's list opened from the layer, a toast) stays live; what is marked
 * `data-layer-keep` is never frozen: the window's own controls, its title bar to drag
 * it by, its edges, the toasts.
 */

interface Layer {
  node: HTMLElement;
  /** Where focus goes back when the layer closes. */
  back: HTMLElement | null;
}

const stack: Layer[] = [];
/** How many layers froze each element of the body; the first one to freeze it remembers it was not inert. */
const frozen = new Map<Element, number>();

/** The focus before the current one: a field that focused itself on mounting the layer took it from its opener. */
let before: HTMLElement | null = null;
let now: HTMLElement | null = null;
if (typeof document !== 'undefined') {
  document.addEventListener('focusin', (e) => {
    before = now;
    now = e.target instanceof HTMLElement ? e.target : null;
  }, true);
}

const TABBABLE = 'a[href], button, input, select, textarea, summary, [tabindex], [contenteditable="true"]';

function tabbables(node: HTMLElement): HTMLElement[] {
  return [...node.querySelectorAll<HTMLElement>(TABBABLE)].filter(
    (el) => el.tabIndex >= 0 && !(el as HTMLButtonElement).disabled && !el.closest('[inert]') && el.getClientRects().length > 0,
  );
}

function freeze(node: HTMLElement): Element[] {
  const held: Element[] = [];
  const keep = [node, ...document.body.querySelectorAll('[data-layer-keep]')];
  // Everything but the layer and what is kept, frozen as high up as it can be: a branch holding
  // neither is frozen whole, one holding them is walked into.
  const walk = (parent: Element) => {
    for (const el of parent.children) {
      if (keep.includes(el) || el.tagName === 'SCRIPT' || el.tagName === 'STYLE') continue;
      if (keep.some((k) => el.contains(k))) {
        walk(el);
        continue;
      }
      const n = frozen.get(el);
      if (n === undefined) {
        if ((el as HTMLElement).inert) continue;
        (el as HTMLElement).inert = true;
      }
      frozen.set(el, (n ?? 0) + 1);
      held.push(el);
    }
  };
  walk(document.body);
  return held;
}

function thaw(held: Element[]) {
  for (const el of held) {
    const n = (frozen.get(el) ?? 1) - 1;
    if (n > 0) frozen.set(el, n);
    else {
      frozen.delete(el);
      (el as HTMLElement).inert = false;
    }
  }
}

function onKeydown(e: KeyboardEvent) {
  const top = stack[stack.length - 1];
  // Shift+Tab may arrive with a different `key` on WebKitGTK; `code` is stable.
  if ((e.key !== 'Tab' && e.code !== 'Tab') || !top || e.defaultPrevented) return;
  const list = tabbables(top.node);
  if (!list.length) {
    e.preventDefault();
    return;
  }
  const at = list.indexOf(document.activeElement as HTMLElement);
  const inside = top.node.contains(document.activeElement);
  if (e.shiftKey ? at <= 0 : at === list.length - 1 || !inside) {
    e.preventDefault();
    (e.shiftKey ? list[list.length - 1] : list[0]).focus();
  }
}

function enter(node: HTMLElement) {
  if (node.contains(document.activeElement)) return;
  const box = node.querySelector<HTMLElement>('[role="dialog"]') ?? node;
  box.focus({ preventScroll: true });
}

export function layer(node: HTMLElement) {
  let back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  if (back && node.contains(back)) back = before;
  const self: Layer = { node, back: back && back !== document.body && !node.contains(back) ? back : null };
  const held = freeze(node);
  if (!stack.length) document.addEventListener('keydown', onKeydown, true);
  stack.push(self);
  enter(node);

  return {
    destroy() {
      const i = stack.indexOf(self);
      const wasTop = i === stack.length - 1;
      stack.splice(i, 1);
      // A layer opened from this one and still open goes back where this one would.
      for (const l of stack.slice(i)) if (l.back && node.contains(l.back)) l.back = self.back;
      thaw(held);
      if (!stack.length) document.removeEventListener('keydown', onKeydown, true);
      if (!wasTop) return;
      const top = stack[stack.length - 1];
      if (self.back?.isConnected && !self.back.closest('[inert]') && (!top || top.node.contains(self.back))) self.back.focus({ preventScroll: true });
      else if (top) enter(top.node);
    },
  };
}
