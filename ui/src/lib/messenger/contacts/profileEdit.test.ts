// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import {
  applyColor, clampCrop, codePoints, initialCrop, layoutOf, maxZoom, panBy, rectOf, sameInput, toggleMark, websiteOf,
  wrapSelection, zoomAt, type TextSelection,
} from './profileEdit';
import type { MessengerProfileInput } from '../api';

/** `[` and `]` mark the selection in a test's text. */
function sel(marked: string): TextSelection {
  const start = marked.indexOf('[');
  const end = marked.indexOf(']') - 1;
  return { value: marked.replace('[', '').replace(']', ''), start, end };
}
function show(s: TextSelection): string {
  return s.value.slice(0, s.start) + '[' + s.value.slice(s.start, s.end) + ']' + s.value.slice(s.end);
}

describe('codePoints', () => {
  it('counts code points, not UTF-16 units', () => {
    expect(codePoints('')).toBe(0);
    expect(codePoints('abc')).toBe(3);
    expect(codePoints('привет')).toBe(6);
    expect(codePoints('😀')).toBe(1);
    expect('😀'.length).toBe(2);
    expect(codePoints('a😀b𝄞')).toBe(4);
    // A family emoji is several code points joined: the runtime counts each.
    expect(codePoints('👨‍👩‍👧')).toBe(5);
  });
});

describe('wrapSelection and toggleMark', () => {
  it('wraps the selection and keeps it selected', () => {
    expect(show(toggleMark(sel('say [hello] now'), 'bold'))).toBe('say **[hello]** now');
    expect(show(toggleMark(sel('[x]'), 'italic'))).toBe('*[x]*');
    expect(show(toggleMark(sel('a [b c] d'), 'strike'))).toBe('a ~~[b c]~~ d');
    expect(show(toggleMark(sel('run [ls -l]'), 'code'))).toBe('run `[ls -l]`');
  });

  it('leaves spaces at the edges outside the marks', () => {
    expect(show(toggleMark(sel('say[ hello ]now'), 'bold'))).toBe('say **[hello]** now');
  });

  it('inserts an empty pair at the caret, and takes it off again', () => {
    const once = toggleMark(sel('ab[]cd'), 'bold');
    expect(show(once)).toBe('ab**[]**cd');
    expect(show(toggleMark(once, 'bold'))).toBe('ab[]cd');
  });

  it('a second press takes the marks off', () => {
    const once = toggleMark(sel('x [word] y'), 'strike');
    expect(show(toggleMark(once, 'strike'))).toBe('x [word] y');
    // The marks inside the selection come off too.
    expect(show(toggleMark(sel('x [**word**] y'), 'bold'))).toBe('x [word] y');
  });

  it('tells italic from bold', () => {
    // Italic around a bold word: both.
    expect(show(toggleMark(sel('**[w]**'), 'italic'))).toBe('***[w]***');
    expect(show(toggleMark(sel('[**w**]'), 'italic'))).toBe('*[**w**]*');
    // Bold off a bold-italic word leaves the italic.
    expect(show(toggleMark(sel('***[w]***'), 'bold'))).toBe('*[w]*');
    expect(show(toggleMark(sel('***[w]***'), 'italic'))).toBe('**[w]**');
  });

  it('keeps the offsets inside the text whatever it is given', () => {
    const r = wrapSelection({ value: 'abc', start: 10, end: 2 }, '`', '`');
    expect(r.value).toBe('ab`c`');
    expect(show(r)).toBe('ab`[c]`');
  });

  it('works across lines and with characters outside the BMP', () => {
    expect(show(toggleMark(sel('[one\ntwo]'), 'bold'))).toBe('**[one\ntwo]**');
    expect(show(toggleMark(sel('a [😀] b'), 'italic'))).toBe('a *[😀]* b');
  });
});

describe('applyColor', () => {
  it('colours, recolours and uncolours', () => {
    const red = applyColor(sel('a [b] c'), 'red');
    expect(show(red)).toBe('a {red}[b]{/} c');
    const blue = applyColor(red, 'blue');
    expect(show(blue)).toBe('a {blue}[b]{/} c');
    const purple = applyColor(blue, 'purple');
    expect(show(purple)).toBe('a {purple}[b]{/} c');
    expect(show(applyColor(purple, 'purple'))).toBe('a [b] c');
  });

  it('inserts an empty pair at the caret', () => {
    expect(show(applyColor(sel('x[]'), 'green'))).toBe('x{green}[]{/}');
  });
});

function close(got: object, want: Record<string, number>) {
  for (const [k, v] of Object.entries(want)) expect((got as Record<string, number>)[k], k).toBeCloseTo(v, 9);
}

describe('crop', () => {
  const W = 1600;
  const H = 1200;

  it('starts with the largest centered square', () => {
    const r = rectOf(initialCrop(), W, H);
    expect(r.w * W).toBeCloseTo(1200);
    expect(r.h * H).toBeCloseTo(1200);
    expect(r.x).toBeCloseTo(0.125);
    expect(r.y).toBeCloseTo(0);
  });

  it('gives a square in pixels, inside the picture, at any zoom and place', () => {
    for (const z of [1, 1.5, 3, 8, 100]) {
      for (const [cx, cy] of [[0, 0], [1, 1], [0.5, 0.5], [-3, 7], [0.9, 0.1]]) {
        const r = rectOf({ cx, cy, z }, W, H);
        expect(r.w * W).toBeCloseTo(r.h * H, 6);
        expect(r.x).toBeGreaterThanOrEqual(0);
        expect(r.y).toBeGreaterThanOrEqual(0);
        expect(r.x + r.w).toBeLessThanOrEqual(1 + 1e-9);
        expect(r.y + r.h).toBeLessThanOrEqual(1 + 1e-9);
      }
    }
  });

  it('works for tall, square and tiny pictures', () => {
    close(rectOf(initialCrop(), 300, 900), { x: 0, y: 1 / 3, w: 1, h: 1 / 3 });
    close(rectOf(initialCrop(), 512, 512), { x: 0, y: 0, w: 1, h: 1 });
    expect(maxZoom(40, 40)).toBe(1);
    close(rectOf({ cx: 0.5, cy: 0.5, z: 5 }, 40, 40), { x: 0, y: 0, w: 1, h: 1 });
  });

  it('bounds the zoom', () => {
    expect(maxZoom(W, H)).toBe(8);
    expect(maxZoom(256, 300)).toBe(4);
    expect(clampCrop({ cx: 0.5, cy: 0.5, z: 0.2 }, W, H).z).toBe(1);
    expect(clampCrop({ cx: NaN, cy: 0.5, z: NaN }, W, H)).toEqual({ cx: 0.5, cy: 0.5, z: 1 });
  });

  it('pans with the finger and stops at the edges', () => {
    const view = 300;
    // At zoom 1 the picture is 400 x 300 in the viewport: 100 px of play sideways, none up and down.
    const start = initialCrop();
    close(layoutOf(start, W, H, view), { width: 400, height: 300, left: -50, top: 0 });
    const right = panBy(start, 30, 0, W, H, view);
    expect(layoutOf(right, W, H, view).left).toBeCloseTo(-20);
    const far = panBy(start, 1000, 1000, W, H, view);
    expect(layoutOf(far, W, H, view).left).toBeCloseTo(0);
    expect(layoutOf(far, W, H, view).top).toBeCloseTo(0);
    const other = panBy(start, -1000, 0, W, H, view);
    expect(layoutOf(other, W, H, view).left).toBeCloseTo(-100);
  });

  it('zooms about a point that stays put', () => {
    const view = 300;
    const start = initialCrop();
    // The point under (100, 120) before and after.
    const before = rectOf(start, W, H);
    const z = zoomAt(start, 2, 100, 120, W, H, view);
    const after = rectOf(z, W, H);
    expect(z.z).toBe(2);
    expect(before.x + (100 / view) * before.w).toBeCloseTo(after.x + (100 / view) * after.w);
    expect(before.y + (120 / view) * before.h).toBeCloseTo(after.y + (120 / view) * after.h);
    // Back to 1: the whole short side again, inside the picture.
    const back = rectOf(zoomAt(z, 1, 0, 0, W, H, view), W, H);
    expect(back.h).toBeCloseTo(1);
    expect(back.x + back.w).toBeLessThanOrEqual(1);
  });
});

describe('form', () => {
  const base: MessengerProfileInput = {
    name: 'alice', display_name: 'Alice', about: 'hi', website: null, nip05: null, lud16: null, socials: [{ p: 'github', h: 'alice' }],
  };

  it('sees no change in spaces at the ends or null for empty', () => {
    expect(sameInput(base, { ...base, name: ' alice ', website: '' })).toBe(true);
    expect(sameInput(base, { ...base, about: 'hi!' })).toBe(false);
    expect(sameInput(base, { ...base, socials: [] })).toBe(false);
    expect(sameInput(base, { ...base, socials: [{ p: 'github', h: 'bob' }] })).toBe(false);
  });

  it('takes a website with http or https, as the runtime does', () => {
    expect(websiteOf('')).toBe(null);
    expect(websiteOf('  example.com ')).toBe('https://example.com');
    expect(websiteOf('https://example.com/a?b')).toBe('https://example.com/a?b');
    expect(websiteOf('http://myblog.example')).toBe('http://myblog.example');
    expect(websiteOf('HTTPS://Example.com')).toBe('https://Example.com');
    for (const bad of ['javascript:alert(1)', 'https://user:pw@example.com', 'localhost', 'exa mple.com', 'ftp://x.org', 'data:text/html,x']) {
      expect(() => websiteOf(bad), bad).toThrow('website_invalid');
    }
  });
});
