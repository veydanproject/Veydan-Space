// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A string the user reads comes from a dictionary in both locales. Russian
// text written straight into markup or code showed Russian in the English UI
// (the notes' version history, the tag and folder dialogs, the proxy and TOTP
// forms); this fails on Cyrillic outside the dictionaries and comments.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..', '..');

/** Files allowed to hold Cyrillic: the dictionaries, test data, dev-only mocks, language names. */
const ALLOWED = [
  /\/i18n\.ts$/,
  /^lib\/notes\/media\/strings\.ts$/,
  /\.test\.ts$/,
  /\/testdata\//,
  // Each language is named in its own language.
  /^lib\/core\/locales\.ts$/,
  /^lib\/core\/(desktop|mobile)\/SettingsPage\.svelte$/,
  // Demo content of the plain-browser dev server.
  /^lib\/messenger\/devDemo\.ts$/,
  /^lib\/messenger\/api\.ts$/,
];

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(svelte|ts)$/.test(name) ? [path] : [];
  });
}

/** The code without its comments (block, HTML and line ones); a block keeps its line breaks. */
const blank = (comment: string) => comment.replace(/[^\n]/g, '');
function code(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/(^|\s)\/\/.*$/gm, '$1');
}

describe('strings outside the dictionaries', () => {
  it('hold no Cyrillic', () => {
    const found: string[] = [];
    for (const path of [...files(join(ROOT, 'lib')), ...files(join(ROOT, 'routes'))]) {
      const rel = relative(ROOT, path).split('\\').join('/');
      if (ALLOWED.some((re) => re.test(rel))) continue;
      code(readFileSync(path, 'utf8'))
        .split('\n')
        .forEach((line, i) => {
          if (/[\u0400-\u04FF]/.test(line)) found.push(`${rel}:${i + 1}: ${line.trim().slice(0, 80)}`);
        });
    }
    expect(found).toEqual([]);
  });
});

// The browser's own dialogs are not the app's: WebKitGTK answered `confirm()`
// without showing anything (a password was deleted unasked), and an Android
// WebView may show nothing at all. Questions go through `ask()` of
// core/ui/confirm.svelte.ts, messages through the page.
describe('browser dialogs', () => {
  it('are never called', () => {
    const found: string[] = [];
    for (const path of [...files(join(ROOT, 'lib')), ...files(join(ROOT, 'routes'))]) {
      const rel = relative(ROOT, path).split('\\').join('/');
      if (/\.test\.ts$/.test(rel)) continue;
      const text = code(readFileSync(path, 'utf8'));
      // A file's own function of that name (a dialog's confirm()) is not the browser's.
      const own = new Set([...text.matchAll(/function\s+(confirm|alert|prompt)\s*\(/g)].map((m) => m[1]));
      text
        .split('\n')
        .forEach((line, i) => {
          // A call of the global, bare or through window, outside a string; not a method.
          const bare = line.replace(/'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`[^`]*`/g, "''");
          const call = /(^|[^.\w$])(window\.)?(confirm|alert|prompt)\s*\(/.exec(bare);
          if (call && !own.has(call[3])) {
            found.push(`${rel}:${i + 1}: ${line.trim().slice(0, 80)}`);
          }
        });
    }
    expect(found).toEqual([]);
  });
});

// English written straight into markup showed English in the Russian UI (the
// workspace board's "Unassigned", "no proxy", "Add column", the tooltips of
// the board and the table, the Raw data panel, "Extracting… please wait", the
// terminal's Maximize/Restore). A name for the reader or a screen reader
// (title, aria-label, placeholder, alt), a text between tags — on one line or
// many, with punctuation — and a word chosen in markup (`x ? 'Restore' :
// 'Maximize'`) come from a dictionary; a few words are names in either language.
describe('English literals in markup', () => {
  /** Product and technical names, the same in every language. */
  const NAMES = new Set(['Blossom', 'Navigator', 'Esc', 'OK', 'Perimeter', 'Ctrl', 'Shift', 'Alt', 'Cmd', 'Enter', 'Tab']);
  /**
   * Whether a text the user reads holds English words. Not counted: names,
   * acronyms (TOTP, JSON, NIP), identifiers (userAgent, WebRTC), single
   * letters, entities; nor a text that is an address, a file or a code
   * (`wss://…`, `user@domain`, `user.js`, `us-east-1`, `SHA256`), nor a
   * single lowercase word — a property name of the Raw data panel.
   */
  function english(text: string): boolean {
    const t = text.replace(/&[a-z]+;|&#\d+;/g, ' ').trim();
    if (/:\/\/|@|^-{3,}/.test(t)) return false;
    if (!/\s/.test(t) && /[._\d-]/.test(t)) return false;
    const all = t.match(/(?<!\w)[A-Za-z][A-Za-z']*(?!\w)/g) ?? [];
    if (all.length === 1 && /^[a-z]+$/.test(all[0])) return false;
    return all.some((w) => w.length > 1 && !NAMES.has(w) && !/^[A-Z\d]+$/.test(w) && !/[a-z][A-Z]/.test(w));
  }
  /** The markup with every `{…}` expression and block tag blanked, line breaks kept. */
  function staticMarkup(markup: string): string {
    let out = markup;
    for (let prev = ''; prev !== out; ) {
      prev = out;
      out = out.replace(/\{[^{}]*\}/g, blank);
    }
    return out;
  }
  const lineAt = (text: string, at: number) => text.slice(0, at).split('\n').length;

  it('are not there', () => {
    const found: string[] = [];
    for (const path of [...files(join(ROOT, 'lib')), ...files(join(ROOT, 'routes'))]) {
      const rel = relative(ROOT, path).split('\\').join('/');
      // The UI Inspector is a developer tool, not a screen of the product.
      if (!rel.endsWith('.svelte') || rel.startsWith('lib/core/inspector/')) continue;
      const markup = code(readFileSync(path, 'utf8')).replace(/<(script|style)[\s\S]*?<\/\1>/g, blank);
      const plain = staticMarkup(markup);
      for (const m of plain.matchAll(/\b(?:title|aria-label|placeholder|alt)="([^"]*)"/g)) {
        if (english(m[1])) found.push(`${rel}:${lineAt(plain, m.index)}: ${m[0]}`);
      }
      // Text nodes: what is left between tags once the expressions are gone.
      for (const m of plain.matchAll(/>([^<>]*)</g)) {
        const text = m[1].replace(/\s+/g, ' ').trim();
        if (text && english(text)) found.push(`${rel}:${lineAt(plain, m.index + 1 + m[1].search(/\S/))}: ${text.slice(0, 80)}`);
      }
      // A word chosen by a condition inside an expression of the markup.
      for (const m of markup.matchAll(/\?\s*(['"])([^'"\n]*)\1\s*:\s*(['"])([^'"\n]*)\3/g)) {
        if ([m[2], m[4]].some((s) => /^[A-Z][a-z]/.test(s) && english(s))) found.push(`${rel}:${lineAt(markup, m.index)}: ${m[0]}`);
      }
    }
    expect(found).toEqual([]);
  });
});

// A hand-rolled full-window backdrop with `inset: 0` covered the window's
// own title bar and shadow gutter on Linux: the window could not be moved or
// closed while it was open (the notes drawer, New note, the media viewer).
// A backdrop takes the frame's tokens (STYLEGUIDE.md, "Modals & drawers").
describe('full-window backdrops', () => {
  /** The frame itself, the start error screen (no frame yet) and the inspector. */
  const FRAME = [
    /^lib\/core\/desktop\/(WindowFrame|ResizeHandles)\.svelte$/,
    /^lib\/core\/StartErrorScreen\.svelte$/,
    /^lib\/core\/inspector\//,
  ];
  it('stay inside the window frame', () => {
    const found: string[] = [];
    for (const path of [...files(join(ROOT, 'lib')), ...files(join(ROOT, 'routes'))]) {
      const rel = relative(ROOT, path).split('\\').join('/');
      if (!rel.endsWith('.svelte') || FRAME.some((re) => re.test(rel))) continue;
      // Phone-only screens have no window frame.
      if (/\/mobile\//.test(rel) || /\/routes\/Mobile|\(mobile\)/.test(rel)) continue;
      const style = /<style[^>]*>([\s\S]*?)<\/style>/.exec(readFileSync(path, 'utf8'))?.[1] ?? '';
      for (const rule of code(style).split('}')) {
        if (/position:\s*fixed/.test(rule) && /(^|[;{\s])inset:\s*0\s*[;}]?/m.test(rule) && !/inset:\s*var\(--overlay-inset/.test(rule)) {
          found.push(`${rel}: ${rule.trim().split('{')[0].trim()}`);
        }
      }
    }
    expect(found).toEqual([]);
  });
});
