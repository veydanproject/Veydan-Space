// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Full-screen viewer state: one picture or video at a time.

export interface ViewerItem {
  messageId: string;
  kind: 'image' | 'video';
  src: string;
  name: string;
}

class Viewer {
  item = $state<ViewerItem | null>(null);

  open(item: ViewerItem) { this.item = item; }
  close() { this.item = null; }
}

export const viewer = new Viewer();
