// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * A block of the shell's at the top of the page on the phone's screen, in its
 * flow (design 18): the first thing of the page's scrolling body under its
 * head, so it scrolls with the page and lies on none of its buttons or its foot.
 * A page without such a body (a chat) takes it under its head; the home its
 * first thing. The page on the screen changes: the block follows it.
 */
export function pageTop(node: HTMLElement) {
  let screen: HTMLElement | null = null;

  const place = () => {
    if (!screen) return;
    const pages = screen.querySelectorAll<HTMLElement>(':scope > .m-page');
    const page = pages[pages.length - 1];
    if (!page) {
      node.remove();
      return;
    }
    const body = page.classList.contains('home') ? page : page.querySelector<HTMLElement>(':scope > .m-body, :scope > .content.scroll');
    if (body) {
      if (body.firstChild !== node) body.prepend(node);
      return;
    }
    const head = page.querySelector<HTMLElement>(':scope > .m-header');
    if (head) {
      if (head.nextSibling !== node) head.after(node);
    } else if (page.firstChild !== node) page.prepend(node);
  };

  // A new page, or a page that redrew the body the block was in.
  const pageSwap = new MutationObserver(() => {
    place();
    watchHost();
  });
  const hostLost = new MutationObserver(() => {
    if (!node.isConnected || node.parentElement !== host) {
      place();
      watchHost();
    }
  });
  let host: HTMLElement | null = null;
  const watchHost = () => {
    if (node.parentElement === host) return;
    hostLost.disconnect();
    host = node.parentElement;
    if (host) hostLost.observe(host, { childList: true });
  };

  // The shell's screen comes with the block or a frame before it (after the unlock).
  let frame = 0;
  const start = () => {
    screen = document.querySelector<HTMLElement>('.shell .screen');
    if (!screen) {
      frame = requestAnimationFrame(start);
      return;
    }
    place();
    watchHost();
    pageSwap.observe(screen, { childList: true });
  };
  start();
  return {
    destroy() {
      cancelAnimationFrame(frame);
      pageSwap.disconnect();
      hostLost.disconnect();
      node.remove();
    },
  };
}
