// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Markdown preview for the note editor. Attachment links
// (`attachments/<note_id>/<name>`) are rewritten to blob URLs.

import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { api } from '$lib/notes/mobile/api';
import { mediaMimeOf } from '$lib/notes/media/kind';

marked.setOptions({ gfm: true, breaks: true });

/** Blob URLs of attachments already read for a note; revoked by `release`. */
export class AttachmentUrls {
  private urls = new Map<string, string>();

  constructor(private noteId: string) {}

  async get(name: string): Promise<string | null> {
    const cached = this.urls.get(name);
    if (cached) return cached;
    try {
      const bytes = await api.attachments.read(this.noteId, name);
      const url = URL.createObjectURL(new Blob([bytes], { type: mimeOf(name) }));
      this.urls.set(name, url);
      return url;
    } catch {
      return null;
    }
  }

  forget(name: string) {
    const url = this.urls.get(name);
    if (url) URL.revokeObjectURL(url);
    this.urls.delete(name);
  }

  release() {
    for (const url of this.urls.values()) URL.revokeObjectURL(url);
    this.urls.clear();
  }
}

function mimeOf(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  const map: Record<string, string> = {
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    gif: 'image/gif',
    webp: 'image/webp',
    bmp: 'image/bmp',
    svg: 'image/svg+xml',
    avif: 'image/avif',
  };
  return map[ext] ?? mediaMimeOf(name) ?? 'application/octet-stream';
}

export { attachmentHref } from '$lib/notes/markdown';

/** Sanitized HTML with attachment images resolved to blob URLs. */
export async function renderMarkdown(md: string, noteId: string, urls: AttachmentUrls): Promise<string> {
  const html = DOMPurify.sanitize(await marked.parse(md));
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const prefix = `attachments/${noteId}/`;
  for (const img of Array.from(doc.querySelectorAll('img'))) {
    const src = decodeURIComponent(img.getAttribute('src') ?? '');
    if (!src.startsWith(prefix)) continue;
    const url = await urls.get(src.slice(prefix.length));
    if (url) img.setAttribute('src', url);
    else img.removeAttribute('src');
  }
  for (const a of Array.from(doc.querySelectorAll('a'))) {
    a.setAttribute('target', '_blank');
    a.setAttribute('rel', 'noopener');
  }
  return doc.body.innerHTML;
}
