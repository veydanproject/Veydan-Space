// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where a place of a textarea's text is on the screen: a list of suggestions
// (Suggest) goes under the line of the caret.

/** The styles a copy of the textarea needs to break its lines the same way. */
const COPIED = [
  'boxSizing', 'width', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
  'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
  'fontStyle', 'fontVariant', 'fontWeight', 'fontStretch', 'fontSize', 'lineHeight', 'fontFamily',
  'textAlign', 'textTransform', 'textIndent', 'letterSpacing', 'wordSpacing', 'tabSize',
  'whiteSpace', 'wordBreak', 'overflowWrap',
] as const;

/** The screen's box of the line at `pos` of the textarea's text, its left edge at `pos`. */
export function textareaCaret(ta: HTMLTextAreaElement, pos: number): { left: number; top: number; bottom: number } {
  const cs = getComputedStyle(ta);
  const copy = document.createElement('div');
  for (const p of COPIED) copy.style[p] = cs[p];
  copy.style.position = 'absolute';
  copy.style.visibility = 'hidden';
  copy.style.top = '0';
  copy.style.left = '-9999px';
  copy.style.overflow = 'hidden';
  copy.style.height = 'auto';
  if (cs.whiteSpace === 'normal') copy.style.whiteSpace = 'pre-wrap';
  copy.textContent = ta.value.slice(0, pos);
  const mark = document.createElement('span');
  mark.textContent = ta.value.slice(pos) || '.';
  copy.appendChild(mark);
  document.body.appendChild(copy);
  const line = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.4;
  const x = mark.offsetLeft;
  const y = mark.offsetTop;
  copy.remove();
  const r = ta.getBoundingClientRect();
  const top = r.top + y - ta.scrollTop;
  return { left: r.left + x - ta.scrollLeft, top, bottom: top + line };
}
