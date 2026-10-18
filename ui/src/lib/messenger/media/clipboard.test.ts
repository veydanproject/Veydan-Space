// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { classifyPaste } from './clipboard';

/** A clipboard as a paste event carries it: data by type, and files. */
function dt(data: Record<string, string>, files: File[] = [], items: { kind: string; file: File | null }[] = []): DataTransfer {
  return {
    types: [...Object.keys(data), ...(files.length ? ['Files'] : [])],
    getData: (t: string) => data[t] ?? '',
    files: files as unknown as FileList,
    items: items.map((i) => ({ kind: i.kind, getAsFile: () => i.file })) as unknown as DataTransferItemList,
  } as unknown as DataTransfer;
}

const png = new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], 'image.png', { type: 'image/png' });

describe('classifyPaste', () => {
  it('takes a screenshot or a copied file when there is no text', () => {
    expect(classifyPaste(dt({}, [png]))).toEqual({ kind: 'files', files: [png] });
    expect(classifyPaste(dt({ 'text/html': '<img src="x">' }, [png]))).toEqual({ kind: 'files', files: [png] });
  });

  it('finds a file the webview gives only among the items', () => {
    expect(classifyPaste(dt({}, [], [{ kind: 'file', file: png }, { kind: 'string', file: null }]))).toEqual({ kind: 'files', files: [png] });
  });

  it('leaves text with a picture of it to the field (Excel, Word)', () => {
    expect(classifyPaste(dt({ 'text/plain': 'A1\tB1' }, [png]))).toEqual({ kind: 'text' });
    expect(classifyPaste(dt({ 'text/plain': 'https://a.example', 'text/uri-list': 'https://a.example' }))).toEqual({ kind: 'text' });
  });

  it('takes files copied in a file manager, whose names come as text too', () => {
    expect(classifyPaste(dt({ 'text/plain': 'report.pdf', 'text/uri-list': 'file:///home/u/report.pdf' }))).toEqual({ kind: 'native' });
    expect(classifyPaste(dt({ 'text/plain': 'image.png', 'text/uri-list': 'file:///tmp/image.png' }, [png]))).toEqual({ kind: 'files', files: [png] });
  });

  it('reads the clipboard itself when the webview only names what it holds (WebKit)', () => {
    expect(classifyPaste(dt({ Files: '' }))).toEqual({ kind: 'native' });
    expect(classifyPaste(dt({ 'text/uri-list': '' }))).toEqual({ kind: 'native' });
    expect(classifyPaste(dt({ 'image/png': '' }))).toEqual({ kind: 'native' });
  });

  it('does nothing with nothing to attach', () => {
    expect(classifyPaste(null)).toEqual({ kind: 'none' });
    expect(classifyPaste(dt({ 'text/plain': '   ' }))).toEqual({ kind: 'none' });
    expect(classifyPaste(dt({ 'text/html': '<b>x</b>' }))).toEqual({ kind: 'none' });
  });
});
