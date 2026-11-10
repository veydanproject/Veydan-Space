// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A modal layer holds the keyboard (layer.ts): the body under it is inert, Tab goes
// round inside it, focus comes back to its opener; a layer over a layer holds it
// until it closes, and the one under it is not thawed before that.

import { afterEach, beforeAll, describe, expect, it } from 'vitest';

class El {
  children: El[] = [];
  parent: El | null = null;
  inert = false;
  tabIndex: number;
  attrs: Record<string, string> = {};
  constructor(public tagName = 'DIV', tabbable = false) {
    this.tabIndex = tabbable ? 0 : -1;
  }
  add(...kids: El[]) {
    for (const k of kids) { k.parent = this; this.children.push(k); }
    return this;
  }
  remove() {
    if (!this.parent) return;
    this.parent.children.splice(this.parent.children.indexOf(this), 1);
    this.parent = null;
  }
  contains(o: unknown): boolean {
    for (let p = o as El | null; p; p = p.parent) if (p === this) return true;
    return false;
  }
  get isConnected() { return doc.body.contains(this); }
  hasAttribute(n: string) { return n in this.attrs; }
  closest(sel: string) {
    for (let p: El | null = this; p; p = p.parent) if ((sel === '[inert]' && p.inert) || (sel === '[role="dialog"]' && p.attrs.role === 'dialog')) return p;
    return null;
  }
  querySelector(sel: string) { return this.all().find((e) => e.closest(sel) === e) ?? null; }
  querySelectorAll(sel: string) { return this.all().filter((e) => (sel === '[data-layer-keep]' ? e.hasAttribute('data-layer-keep') : e.tabIndex >= 0)); }
  all(): El[] { return this.children.flatMap((c) => [c, ...c.all()]); }
  getClientRects() { return [1]; }
  focus() {
    doc.activeElement = this;
    for (const f of doc.focusin) f({ target: this });
  }
}

type Key = { key: string; code?: string; shiftKey?: boolean; defaultPrevented?: boolean; preventDefault: () => void };
const doc = {
  body: new El('BODY'),
  activeElement: null as El | null,
  focusin: [] as ((e: { target: El }) => void)[],
  keydown: [] as ((e: Key) => void)[],
  addEventListener(type: string, f: never) { (type === 'focusin' ? this.focusin : this.keydown).push(f); },
  removeEventListener(type: string, f: never) { if (type === 'keydown') this.keydown = this.keydown.filter((g) => g !== f); },
};

function tab(shiftKey = false, key = 'Tab') {
  const e: Key = { key, code: 'Tab', shiftKey, preventDefault() { e.defaultPrevented = true; } };
  for (const f of doc.keydown) f(e);
  // What the browser does when nobody stopped it: the next one in the layer's order.
  if (!e.defaultPrevented) {
    const all = doc.body.all().filter((x) => x.tabIndex >= 0 && !x.closest('[inert]'));
    all[all.indexOf(doc.activeElement!) + (shiftKey ? -1 : 1)]?.focus();
  }
}

function panel(...kids: El[]) {
  const box = new El();
  box.attrs.role = 'dialog';
  box.add(...kids);
  const backdrop = new El().add(box);
  doc.body.add(backdrop);
  return backdrop;
}

let layer: typeof import('./layer').layer;
let app: El, header: El, row: El;

beforeAll(async () => {
  (globalThis as unknown as { document: typeof doc; HTMLElement: typeof El }).document = doc;
  (globalThis as unknown as { HTMLElement: typeof El }).HTMLElement = El;
  ({ layer } = await import('./layer'));
});

function page() {
  doc.body.children = [];
  header = new El('BUTTON', true);
  row = new El('BUTTON', true);
  app = new El().add(header, row);
  doc.body.add(app);
  row.focus();
}

afterEach(() => { doc.body.children = []; });

describe('layer', () => {
  it('freezes the body, moves focus in, goes round inside and gives focus back', () => {
    page();
    const a = new El('INPUT', true), b = new El('BUTTON', true);
    const node = panel(a, b);
    const l = layer(node as never);
    expect(app.inert).toBe(true);
    expect(node.inert).toBe(false);
    expect(node.contains(doc.activeElement)).toBe(true);
    for (let i = 0; i < 30; i++) {
      tab(i % 3 === 0);
      expect(node.contains(doc.activeElement)).toBe(true);
    }
    l.destroy();
    node.remove();
    expect(app.inert).toBe(false);
    expect(doc.activeElement).toBe(row);
  });

  it('a layer over a layer holds the keyboard until it closes, then the one under it does', () => {
    page();
    const del = new El('BUTTON', true), edit = new El('BUTTON', true);
    const drawer = panel(del, edit);
    const l1 = layer(drawer as never);
    del.focus();
    const yes = new El('BUTTON', true);
    const confirm = panel(new El('BUTTON', true), yes);
    const l2 = layer(confirm as never);
    expect(drawer.inert).toBe(true);
    expect(app.inert).toBe(true);
    for (let i = 0; i < 10; i++) {
      tab();
      expect(confirm.contains(doc.activeElement)).toBe(true);
    }
    l2.destroy();
    confirm.remove();
    expect(drawer.inert).toBe(false);
    expect(app.inert).toBe(true);
    expect(doc.activeElement).toBe(del);
    l1.destroy();
    drawer.remove();
    expect(app.inert).toBe(false);
    expect(doc.activeElement).toBe(row);
  });

  it("the window keeps its own controls under a layer, not the top bar's buttons beside them", () => {
    page();
    const controls = new El();
    controls.attrs['data-layer-keep'] = '';
    const tools = new El().add(new El('BUTTON', true));
    const titlebar = new El().add(tools, controls);
    app.add(titlebar);
    const l = layer(panel(new El('BUTTON', true)) as never);
    expect(controls.closest('[inert]')).toBe(null);
    expect(tools.inert).toBe(true);
    expect(header.inert).toBe(true);
    l.destroy();
    expect(tools.inert).toBe(false);
  });

  it('what the body gets later and a toast stay live; what was inert before stays inert', () => {
    page();
    const toast = new El();
    toast.attrs['data-layer-keep'] = '';
    const hidden = new El();
    hidden.inert = true;
    doc.body.add(toast, hidden);
    const node = panel(new El('BUTTON', true));
    const l = layer(node as never);
    const menu = new El();
    doc.body.add(menu);
    expect(toast.inert).toBe(false);
    expect(menu.inert).toBe(false);
    l.destroy();
    expect(hidden.inert).toBe(true);
  });

  it("Shift+Tab with WebKitGTK's own key does not leave a layer for the window's controls", () => {
    page();
    const controls = new El();
    controls.attrs['data-layer-keep'] = '';
    const close = new El('BUTTON', true);
    controls.add(close);
    app.add(controls);
    const x = new El('BUTTON', true), cancel = new El('BUTTON', true);
    const node = panel(x, cancel);
    const l = layer(node as never);
    x.focus();
    tab(true, 'Unidentified');
    expect(doc.activeElement).toBe(cancel);
    for (let i = 0; i < 10; i++) {
      tab(true, 'Unidentified');
      expect(node.contains(doc.activeElement)).toBe(true);
    }
    l.destroy();
  });

  it('a field that focused itself on opening still gives focus back to the opener', () => {
    page();
    const field = new El('INPUT', true);
    const node = panel(field);
    field.focus();
    const l = layer(node as never);
    expect(doc.activeElement).toBe(field);
    l.destroy();
    node.remove();
    expect(doc.activeElement).toBe(row);
  });
});
