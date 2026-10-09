// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { canEdit, editTarget } from './editable';

const msg = (over: Partial<Parameters<typeof canEdit>[0]> = {}) =>
  ({ id: 'm1', direction: 'out', status: 'sent', content_type: 'text', deleted: false, ...over }) as Parameters<typeof canEdit>[0];

describe('canEdit', () => {
  it('takes my own text', () => {
    expect(canEdit(msg(), true)).toBe(true);
  });

  it('takes the caption of my own photo or file once it has left', () => {
    expect(canEdit(msg({ content_type: 'media' }), true)).toBe(true);
    expect(canEdit(msg({ content_type: 'media', status: 'queued' }), true)).toBe(true);
  });

  it('leaves an upload on its way, a paused one and a failed one alone', () => {
    expect(canEdit(msg({ content_type: 'media', status: 'uploading', id: 'm9' }), true)).toBe(false);
    expect(canEdit(msg({ content_type: 'media', status: 'paused', id: 'm9' }), true)).toBe(false);
    expect(canEdit(msg({ content_type: 'media', status: 'failed', id: 'm9' }), true)).toBe(false);
    expect(canEdit(msg({ content_type: 'media', id: 'local:1' }), true)).toBe(false);
  });

  it('does not take somebody else\'s message, a card, a sticker or a deleted one', () => {
    expect(canEdit(msg({ direction: 'in' }), true)).toBe(false);
    expect(canEdit(msg({ direction: 'in', content_type: 'media' }), true)).toBe(false);
    expect(canEdit(msg({ content_type: 'contact' }), true)).toBe(false);
    expect(canEdit(msg({ content_type: 'sticker' }), true)).toBe(false);
    expect(canEdit(msg({ deleted: true }), true)).toBe(false);
  });

  it('offers nothing where I may not write', () => {
    expect(canEdit(msg(), false)).toBe(false);
    expect(canEdit(msg({ content_type: 'media' }), false)).toBe(false);
  });
});

describe('editTarget', () => {
  const part = (id: string, text: string | null) => ({ id, text });

  it('is the pressed message when it stands alone', () => {
    const m = part('a', 'hi');
    expect(editTarget(m)).toBe(m);
    expect(editTarget(m, [])).toBe(m);
  });

  it('is the part that carries the caption, whichever part is pressed', () => {
    const album = [part('a', null), part('b', 'look'), part('c', null)];
    expect(editTarget(album[0], album)).toBe(album[1]);
    expect(editTarget(album[2], album)).toBe(album[1]);
  });

  it('takes the last caption when several parts carry one', () => {
    const album = [part('a', 'one'), part('b', 'two'), part('c', ' ')];
    expect(editTarget(album[0], album)).toBe(album[1]);
  });

  it('is the last part of an album without a caption', () => {
    const album = [part('a', null), part('b', null)];
    expect(editTarget(album[0], album)).toBe(album[1]);
  });
});
