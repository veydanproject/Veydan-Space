// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The tooltips of a computer (design kit, tooltip): every `title` of the
// page shows as the kit's glass box instead of the system's, without a change
// at the places that set it. A mouse or a pen resting on the element 500 ms,
// or the keyboard's focus at once; below the element, centred, 6 apart, above
// it when there is no room, 8 inside the window. Esc, a press, the wheel or
// leaving hides it. A shortcut at the end of the text — «Найти (Ctrl+F)» —
// becomes a key chip. While the box is up the element's `title` waits in
// `data-tip`, so the system shows nothing of its own; it comes back after.
// Touch has no tooltips. Installed once by the desktop shell.

const DELAY = 500;
const GAP = 6;
const EDGE = 8;
const SHORTCUT = /^(.*\S)\s+\(((?:Ctrl|Cmd|⌘|⌥|⇧|Shift|Alt|Esc|Del|Enter|Tab|F\d{1,2})(?:[+ ][^()]*)?)\)$/;

export function installTooltips(): () => void {
  const box = document.createElement('div');
  box.className = 'vtip';
  box.setAttribute('role', 'tooltip');
  box.setAttribute('aria-hidden', 'true');
  box.hidden = true;
  document.body.appendChild(box);

  let el: HTMLElement | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;

  /** Takes the element's title into data-tip (the system then shows nothing). */
  const take = (node: HTMLElement): string => {
    const title = node.getAttribute('title');
    if (title !== null) {
      node.dataset.tip = title;
      node.removeAttribute('title');
    }
    return node.dataset.tip ?? '';
  };

  /** Gives the title back, unless the page has set a new one meanwhile. */
  const give = (node: HTMLElement) => {
    const tip = node.dataset.tip;
    delete node.dataset.tip;
    if (tip !== undefined && !node.hasAttribute('title')) node.setAttribute('title', tip);
  };

  function hide() {
    clearTimeout(timer);
    box.hidden = true;
  }

  function leave() {
    hide();
    if (el) give(el);
    el = null;
  }

  function place(node: HTMLElement) {
    const r = node.getBoundingClientRect();
    const w = box.offsetWidth;
    const h = box.offsetHeight;
    let top = r.bottom + GAP;
    if (top + h > window.innerHeight - EDGE && r.top - GAP - h >= EDGE) top = r.top - GAP - h;
    const left = Math.max(EDGE, Math.min(r.left + r.width / 2 - w / 2, window.innerWidth - EDGE - w));
    box.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
  }

  function show(node: HTMLElement) {
    if (el !== node || !node.isConnected) return;
    const text = node.dataset.tip ?? '';
    if (!text.trim()) return;
    const m = SHORTCUT.exec(text);
    box.replaceChildren(document.createTextNode(m ? m[1] : text));
    if (m) {
      const kbd = document.createElement('span');
      kbd.className = 'vtip-kbd';
      kbd.textContent = m[2];
      box.appendChild(kbd);
    }
    box.hidden = false;
    place(node);
  }

  function enter(node: HTMLElement, now: boolean) {
    if (el === node) return;
    leave();
    if (!take(node).trim()) { give(node); return; }
    el = node;
    if (now) show(node);
    else timer = setTimeout(() => show(node), DELAY);
  }

  const target = (n: EventTarget | null) =>
    n instanceof Element ? n.closest<HTMLElement>('[title], [data-tip]') : null;

  const over = (e: PointerEvent) => {
    if (e.pointerType === 'touch') return;
    const node = target(e.target);
    if (node) enter(node, false);
    else if (el && !el.contains(e.target as Node)) leave();
  };
  const out = (e: PointerEvent) => {
    if (el && !(e.relatedTarget instanceof Node && el.contains(e.relatedTarget))) leave();
  };
  const focusIn = (e: FocusEvent) => {
    const node = e.target instanceof HTMLElement ? e.target : null;
    if (node && node.matches(':focus-visible') && (node.hasAttribute('title') || node.dataset.tip !== undefined)) enter(node, true);
  };
  const focusOut = (e: FocusEvent) => {
    if (el && e.target === el) leave();
  };
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && !box.hidden) hide();
  };

  document.addEventListener('pointerover', over, true);
  document.addEventListener('pointerout', out, true);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('focusin', focusIn, true);
  document.addEventListener('focusout', focusOut, true);
  document.addEventListener('keydown', key, true);
  window.addEventListener('wheel', hide, { capture: true, passive: true });
  window.addEventListener('blur', leave);

  return () => {
    leave();
    document.removeEventListener('pointerover', over, true);
    document.removeEventListener('pointerout', out, true);
    document.removeEventListener('pointerdown', hide, true);
    document.removeEventListener('focusin', focusIn, true);
    document.removeEventListener('focusout', focusOut, true);
    document.removeEventListener('keydown', key, true);
    window.removeEventListener('wheel', hide, { capture: true });
    window.removeEventListener('blur', leave);
    box.remove();
  };
}
