// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export function portal(node: HTMLElement) {
  document.body.appendChild(node);
  return {
    destroy() {
      if (document.body.contains(node)) {
        document.body.removeChild(node);
      }
    },
  };
}
