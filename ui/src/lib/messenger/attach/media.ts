// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** The extensions the chooser of photos and videos offers. */
export const MEDIA_EXTENSIONS = {
  image: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'heif', 'avif', 'bmp'],
  video: ['mp4', 'mov', 'm4v', 'webm', 'mkv', '3gp'],
  get media() { return [...this.image, ...this.video]; },
};
