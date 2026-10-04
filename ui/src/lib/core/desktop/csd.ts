// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Client-side decorations are used only on Linux (to dodge the KWin
// hide()/show() decoration bug): there the window is transparent and the UI
// draws its frame, title bar and controls (WindowFrame.svelte). Windows and
// macOS keep their native window chrome.

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const isCsd = isTauri && typeof navigator !== 'undefined' && /linux/i.test(navigator.userAgent);

/** The frame as WindowFrame.svelte draws it; the overlays' inset follows these. */
export const FRAME = { gutter: 9, radius: 11, titlebar: 34 } as const;
