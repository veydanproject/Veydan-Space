// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which phone the page runs on, for what the gallery plugin does on both
// (tauri-plugin-veydan-media: the grid, the camera, the share sheet, the
// photo library). The messenger may not ask $lib/core/platform, so it reads
// the user agent, as it always has.

/** The phone's system: `android`, `ios`, or `other` for a computer and a browser. */
export type PhoneOs = 'android' | 'ios' | 'other';

/**
 * The system of a user agent. An iPad says it is a Mac (the desktop-class
 * WebKit of iPadOS 13+): told apart by its touch points, which a Mac has none of.
 */
export function phoneOs(userAgent: string, touchPoints = 0): PhoneOs {
  if (/android/i.test(userAgent)) return 'android';
  if (/iPhone|iPad|iPod/.test(userAgent)) return 'ios';
  if (/Macintosh/.test(userAgent) && touchPoints > 1) return 'ios';
  return 'other';
}

/** The system of this page. */
export const PHONE_OS: PhoneOs =
  typeof navigator === 'undefined' ? 'other' : phoneOs(navigator.userAgent, navigator.maxTouchPoints ?? 0);

/** A phone with the gallery plugin: Android or iOS. */
export const onPhone = PHONE_OS !== 'other';
