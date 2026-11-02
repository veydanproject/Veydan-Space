// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { readFileSync } from 'node:fs';
import jsQR from 'jsqr';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { logoDisc, qrPicture, svgSrc } from './qrPicture';

// A link and the picture of its code as the core draws it
// (crates/messenger/links/src/qr.rs keeps the file equal to what it draws).
const [LINK, SVG] = readFileSync(new URL('../../../../../crates/messenger/links/testdata/qr.txt', import.meta.url), 'utf8').split('\n');

type Rect = { x: number; y: number; w: number; h: number; r: [number, number, number, number]; dark: boolean };

/** The shapes of the picture, in order: its rects, the subpaths of its path (rounded rects too), its disc. */
function shapes(svg: string): { rects: Rect[]; disc: { cx: number; cy: number; r: number } } {
  const rects: Rect[] = [];
  let disc = { cx: 0, cy: 0, r: 0 };
  for (const [el] of svg.matchAll(/<(rect|path|circle)[^>]*>/g)) {
    const attr = (name: string) => new RegExp(` ${name}="([^"]*)"`).exec(el)![1];
    const dark = attr('fill') !== '#ffffff';
    if (el.startsWith('<rect')) {
      const r = Number(attr('rx'));
      rects.push({ x: Number(attr('x')), y: Number(attr('y')), w: Number(attr('width')), h: Number(attr('height')), r: [r, r, r, r], dark });
    } else if (el.startsWith('<circle')) {
      disc = { cx: Number(attr('cx')), cy: Number(attr('cy')), r: Number(attr('r')) };
    } else {
      // M x y h · [a] v · [a] h · [a] v · [a] z: the arcs, if any, are the
      // corners top right, bottom right, bottom left, top left.
      for (const sub of attr('d').split('z').filter(Boolean)) {
        const t = sub.match(/[a-zA-Z]|-?(?:\d+\.?\d*|\.\d+)/g)!;
        let [px, py] = [Number(t[1]), Number(t[2])];
        const xs = [px], ys = [py];
        const r: [number, number, number, number] = [0, 0, 0, 0];
        let side = 0;
        for (let i = 3; i < t.length; ) {
          const c = t[i];
          if (c === 'h') { px += Number(t[i + 1]); side++; i += 2; }
          else if (c === 'v') { py += Number(t[i + 1]); side++; i += 2; }
          else if (c === 'a') { r[side % 4] = Number(t[i + 1]); px += Number(t[i + 6]); py += Number(t[i + 7]); i += 8; }
          else throw new Error(`path: ${c}`);
          xs.push(px); ys.push(py);
        }
        const [x, y] = [Math.min(...xs), Math.min(...ys)];
        rects.push({ x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y, r: [r[0], r[1], r[2], r[3]], dark });
      }
    }
  }
  return { rects, disc };
}

function inside(fx: number, fy: number, { x, y, w, h, r: [tl, tr, br, bl] }: Rect): boolean {
  if (fx < x || fy < y || fx > x + w || fy > y + h) return false;
  const corner = (cx: number, cy: number, r: number) => r === 0 || (fx - cx) ** 2 + (fy - cy) ** 2 <= r * r;
  if (fx < x + tl && fy < y + tl) return corner(x + tl, y + tl, tl);
  if (fx > x + w - tr && fy < y + tr) return corner(x + w - tr, y + tr, tr);
  if (fx > x + w - br && fy > y + h - br) return corner(x + w - br, y + h - br, br);
  if (fx < x + bl && fy > y + h - bl) return corner(x + bl, y + h - bl, bl);
  return true;
}

/**
 * The picture as `readQrImage` gets a saved PNG: `k` pixels a module, its
 * clear corners black (transparent pixels are black to jsQR), the disc
 * dark all over (the worst a logo can do).
 */
function raster(svg: string, k: number): { data: Uint8ClampedArray; side: number } {
  const side = Number(/viewBox="0 0 ([\d.]+)/.exec(svg)![1]) * k;
  const grey = new Uint8Array(side * side);
  const { rects, disc } = shapes(svg);
  const paint = (x: number, y: number, w: number, h: number, v: (fx: number, fy: number) => number | null) => {
    for (let py = Math.floor(y * k); py < Math.min(side, Math.ceil((y + h) * k)); py++)
      for (let px = Math.floor(x * k); px < Math.min(side, Math.ceil((x + w) * k)); px++) {
        const got = v((px + 0.5) / k, (py + 0.5) / k);
        if (got !== null) grey[py * side + px] = got;
      }
  };
  for (const rect of rects) paint(rect.x, rect.y, rect.w, rect.h, (fx, fy) => (inside(fx, fy, rect) ? (rect.dark ? 0 : 255) : null));
  const { cx, cy, r } = disc;
  paint(cx - r, cy - r, 2 * r, 2 * r, (fx, fy) => ((fx - cx) ** 2 + (fy - cy) ** 2 <= r * r ? 40 : null));
  const data = new Uint8ClampedArray(side * side * 4);
  grey.forEach((v, i) => { data.set([v, v, v, 255], i * 4); });
  return { data, side };
}

describe('qrPicture', () => {
  it('a code with the logo, rounded eyes and all, reads back as its link', () => {
    expect(LINK.startsWith('veydan://group/')).toBe(true);
    // The card, nine rects of the eyes and the runs of modules.
    expect(shapes(SVG).rects.length).toBeGreaterThan(500);
    for (const k of [3, 5]) {
      const { data, side } = raster(SVG, k);
      expect(jsQR(data, side, side)?.data).toBe(LINK);
    }
  });

  it('finds the disc of the logo in the middle of the picture', () => {
    const disc = logoDisc(SVG)!;
    expect(disc.x).toBeCloseTo(0.5);
    expect(disc.y).toBeCloseTo(0.5);
    expect(disc.r).toBeGreaterThan(0.08);
    expect(disc.r).toBeLessThan(0.15);
    expect(logoDisc('<svg viewBox="0 0 21 21"></svg>')).toBeNull();
  });

  it('shows an SVG as an image', () => {
    expect(svgSrc('<svg/>')).toBe(`data:image/svg+xml;base64,${btoa('<svg/>')}`);
  });

  describe('where the canvas fails', () => {
    afterEach(() => { vi.unstubAllGlobals(); });

    /** Pictures that load (`decode` as given) and a canvas with no 2D context. */
    function stub(decode: () => Promise<void>) {
      vi.stubGlobal('Image', class { src = ''; naturalWidth = 1; naturalHeight = 1; decode = decode; });
      vi.stubGlobal('document', { createElement: () => ({ width: 0, height: 0, getContext: () => null }) });
    }

    it('shows the code as drawn when there is no 2D context', async () => {
      stub(() => Promise.resolve());
      await expect(qrPicture(SVG)).resolves.toBe(svgSrc(SVG));
    });

    it('shows the code as drawn when the picture does not decode', async () => {
      stub(() => Promise.reject(new Error('EncodingError')));
      await expect(qrPicture(SVG)).resolves.toBe(svgSrc(SVG));
    });
  });
});
