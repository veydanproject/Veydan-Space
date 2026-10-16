// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A QR code read from a picture: the desktop's way where a phone has its
// camera (QrScanner). In the core, as the scanner is, so a module reads a
// code without a library of its own.

/** The text of the QR code in the picture; `null` when there is none. */
export async function readQrImage(file: Blob): Promise<string | null> {
  const bitmap = await createImageBitmap(file);
  const canvas = document.createElement('canvas');
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(bitmap, 0, 0);
  bitmap.close();
  const image = ctx.getImageData(0, 0, canvas.width, canvas.height);
  const jsQR = (await import('jsqr')).default;
  return jsQR(image.data, image.width, image.height)?.data ?? null;
}
