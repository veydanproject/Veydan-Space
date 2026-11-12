// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Whether the keyboard moves the focus: `kbd-focus` on <html> from a Tab
 * until the next press of the pointer. A text field is `:focus-visible`
 * under the mouse as well, so its keyboard ring (base.css) asks for this.
 * Returns what takes the listeners off.
 */
export function installFocusMode(): () => void {
  const root = document.documentElement;
  const key = (e: KeyboardEvent) => {
    if (e.key === 'Tab') root.classList.add('kbd-focus');
  };
  const pointer = () => root.classList.remove('kbd-focus');
  window.addEventListener('keydown', key, true);
  window.addEventListener('pointerdown', pointer, true);
  return () => {
    window.removeEventListener('keydown', key, true);
    window.removeEventListener('pointerdown', pointer, true);
    root.classList.remove('kbd-focus');
  };
}
