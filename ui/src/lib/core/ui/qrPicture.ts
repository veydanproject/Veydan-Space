// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A QR code as the app shows and saves it (ROO-462). The core draws the
// code (messenger_links::qr: a white card, rounded modules and eyes, a
// dark disc in the middle); here the logo of this build's product goes
// into the disc: `/logo.png`, Space's in Space and Chat's in Chat
// (vite-veydan-modules.js, BRAND). Every screen of a code shows the
// picture made here and «Сохранить» saves the same one, so what is saved is
// what is seen. In the core, as the reader of a picture (qrImage.ts) is.

/** The side of the picture: sharp on a phone's 3x screen, in print and on a big screen, still small. */
export const QR_SIDE = 1024;

/** `<img src>` of an SVG: an image, nothing inside it can run. */
export function svgSrc(svg: string): string {
  return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
}

/** The disc of the logo, its centre and radius as shares of the side; null when the picture has none. */
export function logoDisc(svg: string): { x: number; y: number; r: number } | null {
  const box = /viewBox="0 0 ([\d.]+) ([\d.]+)"/.exec(svg);
  const disc = /<circle class="logo" cx="([\d.]+)" cy="([\d.]+)" r="([\d.]+)"/.exec(svg);
  if (!box || !disc) return null;
  const side = Number(box[1]);
  const [x, y, r] = disc.slice(1).map((v) => Number(v) / side);
  return side > 0 && r > 0 ? { x, y, r } : null;
}

function image(src: string): Promise<HTMLImageElement> {
  const img = new Image();
  img.src = src;
  return img.decode().then(() => img);
}

let logo: Promise<HTMLImageElement | null> | null = null;

/** The product's logo, loaded once; null where there is none: the disc stays plain. */
function productLogo(): Promise<HTMLImageElement | null> {
  logo ??= image('/logo.png').catch(() => null);
  return logo;
}

/** The share of the disc the logo takes, a ring of the disc around it (as the example of ROO-462). */
const LOGO_SHARE = 0.8;

/** The code drawn on a canvas of `side` pixels, the logo in its disc; clear around the card. */
export async function qrCanvas(svg: string, side = QR_SIDE): Promise<HTMLCanvasElement> {
  const [code, mark] = await Promise.all([image(svgSrc(svg)), productLogo()]);
  const canvas = document.createElement('canvas');
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('no canvas');
  ctx.drawImage(code, 0, 0, side, side);
  const disc = logoDisc(svg);
  if (mark && disc) {
    const d = 2 * disc.r * side * LOGO_SHARE;
    const scale = Math.min(d / mark.naturalWidth, d / mark.naturalHeight);
    const [w, h] = [mark.naturalWidth * scale, mark.naturalHeight * scale];
    ctx.save();
    ctx.beginPath();
    ctx.arc(disc.x * side, disc.y * side, disc.r * side, 0, 2 * Math.PI);
    ctx.clip();
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(mark, disc.x * side - w / 2, disc.y * side - h / 2, w, h);
    ctx.restore();
  }
  return canvas;
}

/**
 * `<img src>` of the code as the screens show it: the picture «Сохранить»
 * saves. Where the canvas fails (no 2D context, an SVG it cannot decode, no
 * PNG of it) the code is shown as drawn, without the logo: a code still,
 * never a screen waiting for it.
 */
export async function qrPicture(svg: string): Promise<string> {
  try {
    return (await qrCanvas(svg)).toDataURL('image/png');
  } catch {
    return svgSrc(svg);
  }
}

/** The code as a PNG of `side` pixels. */
export async function qrPng(svg: string, side = QR_SIDE): Promise<Uint8Array> {
  const canvas = await qrCanvas(svg, side);
  const blob = await new Promise<Blob>((ok, fail) => canvas.toBlob((b) => (b ? ok(b) : fail(new Error('no png'))), 'image/png'));
  return new Uint8Array(await blob.arrayBuffer());
}
