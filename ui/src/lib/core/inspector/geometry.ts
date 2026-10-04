// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export interface ElementInfo {
  width: number;
  height: number;
  top: number;
  left: number;
  paddingTop: number;
  paddingRight: number;
  paddingBottom: number;
  paddingLeft: number;
  marginTop: number;
  marginRight: number;
  marginBottom: number;
  marginLeft: number;
  zIndex: string;
  tagName: string;
  id: string;
  classList: string;
}

export function getElementInfo(el: Element): ElementInfo {
  const rect = el.getBoundingClientRect();
  const style = getComputedStyle(el);
  return {
    width: Math.round(rect.width),
    height: Math.round(rect.height),
    top: Math.round(rect.top),
    left: Math.round(rect.left),
    paddingTop: parseFloat(style.paddingTop),
    paddingRight: parseFloat(style.paddingRight),
    paddingBottom: parseFloat(style.paddingBottom),
    paddingLeft: parseFloat(style.paddingLeft),
    marginTop: parseFloat(style.marginTop),
    marginRight: parseFloat(style.marginRight),
    marginBottom: parseFloat(style.marginBottom),
    marginLeft: parseFloat(style.marginLeft),
    zIndex: style.zIndex,
    tagName: el.tagName.toLowerCase(),
    id: el.id,
    classList: Array.from(el.classList).slice(0, 3).join(' '),
  };
}

export function getElementAtPoint(x: number, y: number): Element | null {
  const els = document.elementsFromPoint(x, y);
  for (const el of els) {
    if (el.hasAttribute('data-inspector-ui')) continue;
    if (el === document.body || el === document.documentElement) continue;
    return el;
  }
  return null;
}
