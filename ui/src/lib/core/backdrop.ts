// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Close an overlay only when the pointer went down and came up on the backdrop itself.
 * A drag that starts inside the dialog (text selection) and ends outside fires `click`
 * on the backdrop too; that one is ignored.
 */
export function backdropDismiss(node: HTMLElement, onDismiss: () => void) {
  let handler = onDismiss;
  let armed = false;

  const onPointerDown = (e: PointerEvent) => {
    armed = e.target === node;
  };
  const onClick = (e: MouseEvent) => {
    const hit = armed && e.target === node;
    armed = false;
    if (hit) handler();
  };

  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('click', onClick);
  return {
    update(next: () => void) {
      handler = next;
    },
    destroy() {
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('click', onClick);
    },
  };
}
