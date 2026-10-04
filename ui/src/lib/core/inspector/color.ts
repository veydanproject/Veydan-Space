// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export async function pickColor(): Promise<string | null> {
  if ('EyeDropper' in window) {
    try {
      // @ts-expect-error EyeDropper is not yet in TS lib
      const dropper = new window.EyeDropper();
      const result = await dropper.open();
      return result.sRGBHex as string;
    } catch {
      return null;
    }
  }
  return null;
}

export function getElementColor(el: Element): string {
  return getComputedStyle(el).backgroundColor;
}
