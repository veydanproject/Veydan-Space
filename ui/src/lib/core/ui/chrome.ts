// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone's bridge `VeydanChrome` (platform-spec 13.6), the one place the
// page looks it up. Android's shell injects it into the WebView as an
// object (crates/shell/android, ActivityDelegate); the iPhone's shell has
// commands instead (crates/shell/src/commands/chrome.rs), shown here under
// the same names. A computer and a browser have neither.

import { call } from '$lib/core/api';
import { platform } from '$lib/core/platform';

export interface ChromeBridge {
  /** Dark icons of the bars on a light theme, light ones on a dark theme. */
  setLightBars?: (light: boolean) => void;
  /** Whether the clipboard holds a picture (only its description is read: the system says nothing). */
  clipboardHasImage?: () => boolean;
  /**
   * The picture on the clipboard, read on the user's tap only (Android 12+
   * shows a toast, iOS asks «Allow Paste»): a `content://` URI on Android,
   * the path of a copy in the app's cache on iOS; null without one.
   */
  clipboardImage?: () => string | null | Promise<string | null>;
  /** Whether pictures a keyboard inserts come to the page as `veydan-received-content` (Android). */
  setTakesPictures?: (takes: boolean) => void;
  /** Where the page handles a drag from the edge of the screen itself (CSS pixels); an empty rectangle gives it back. */
  setGestureExclusion?: (left: number, top: number, width: number, height: number) => void;
}

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** `UIRectEdge` of the edges a rectangle of the page reaches: 1 top, 2 left, 4 bottom, 8 right. */
export function edgesOf(left: number, top: number, width: number, height: number, viewW: number, viewH: number): number {
  if (width <= 0 || height <= 0) return 0;
  const near = 1;
  return (top <= near ? 1 : 0) | (left <= near ? 2 : 0) | (top + height >= viewH - near ? 4 : 0) | (left + width >= viewW - near ? 8 : 0);
}

function iosBridge(): ChromeBridge {
  // Whether the clipboard holds a picture, asked anew when the app comes to
  // the front (a copy is made in another app) and on each question: the
  // answer of the question before is the one given.
  let hasImage = false;
  const refresh = () => {
    call<boolean>('chrome_clipboard_has_image').then((has) => { hasImage = has; }, () => { hasImage = false; });
  };
  refresh();
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
  window.addEventListener('copy', () => setTimeout(refresh, 0));
  let edges = -1;
  return {
    setLightBars(light) {
      // The custom property, not the body's colour: that one is in its transition.
      const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
      void call('chrome_bars', { light, background: /^#[0-9a-f]{6}$/i.test(bg) ? bg : null }).catch(() => {});
    },
    clipboardHasImage() {
      refresh();
      return hasImage;
    },
    clipboardImage: () => call<string | null>('chrome_clipboard_image').catch(() => null),
    setGestureExclusion(left, top, width, height) {
      const next = edgesOf(left, top, width, height, window.innerWidth, window.innerHeight);
      if (next === edges) return;
      edges = next;
      void call('chrome_edges', { edges: next }).catch(() => {});
    },
  };
}

let ios: ChromeBridge | undefined;

/** The bridge of this phone's shell, if the page runs in one. */
export function chromeBridge(): ChromeBridge | undefined {
  if (typeof window === 'undefined') return undefined;
  const injected = (window as Window & { VeydanChrome?: ChromeBridge }).VeydanChrome;
  if (injected) return injected;
  if (platform !== 'ios' || !inTauri) return undefined;
  ios ??= iosBridge();
  return ios;
}
