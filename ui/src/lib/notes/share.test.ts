// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { noteOf } from './share';

const file = (name: string) => ({ path: `/cache/share-inbox/1/${name}`, name, mime: null, size: 1 });

describe('a share from another app as a note', () => {
  it('takes the subject as the title and the whole text as the body', () => {
    expect(noteOf({ text: 'https://x.example/p\nsee this', subject: 'A page', files: [] }, 'Shared'))
      .toEqual({ title: 'A page', content: 'https://x.example/p\nsee this' });
  });

  it('takes the first line of the text as the title, the rest as the body', () => {
    expect(noteOf({ text: '\n\n# Idea\r\nline 2\nline 3\n', subject: null, files: [] }, 'Shared'))
      .toEqual({ title: 'Idea', content: 'line 2\nline 3' });
    expect(noteOf({ text: 'just one line', subject: null, files: [] }, 'Shared'))
      .toEqual({ title: 'just one line', content: '' });
    expect(noteOf({ text: 'x'.repeat(200), subject: null, files: [] }, 'Shared').title).toHaveLength(120);
  });

  it('names a note of files alone by the first file, else as untitled', () => {
    expect(noteOf({ text: null, subject: null, files: [file('IMG_1.jpg'), file('IMG_2.jpg')] }, 'Shared'))
      .toEqual({ title: 'IMG_1.jpg', content: '' });
    expect(noteOf({ text: '  ', subject: null, files: [] }, 'Shared')).toEqual({ title: 'Shared', content: '' });
  });
});
