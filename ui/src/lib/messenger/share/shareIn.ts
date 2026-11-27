// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What another app shared into Veydan (`core/shares`), as the messenger
// takes it: the decisions of the chooser of a chat (ShareIn.svelte) and of
// the chat that opens with the share, without a store or a screen.

import type { IncomingShare, SharedFile } from '$lib/core/module';
import type { MessengerChat } from '../api';
import { forwardTarget, MAX_FORWARD_TARGETS } from '../dm/select';

/** The most chats one share goes to at once: the forward's limit. */
export const MAX_SHARE_TARGETS = MAX_FORWARD_TARGETS;

/** A chat a share may go to: the same chats a forward may go to. */
export const shareTarget = forwardTarget;

/**
 * The text the composer of the chosen chat opens with: the subject (a page's
 * title, a mail's) above the text when both came; nothing for files alone.
 */
export function draftOf(share: Pick<IncomingShare, 'text' | 'subject'>): string {
  const parts = [share.subject?.trim(), share.text?.trim()].filter((p): p is string => !!p);
  return parts.join('\n');
}

/**
 * The caption of the last file when the share goes straight to several
 * chats: its text, and the comment typed in the chooser under it.
 */
export function captionOf(share: Pick<IncomingShare, 'text' | 'subject'>, comment: string): string | undefined {
  const text = [draftOf(share), comment.trim()].filter(Boolean).join('\n');
  return text || undefined;
}

/**
 * The chat the system's sheet chose itself (Direct Share), when it is one
 * the share may go to now; otherwise the chooser is shown.
 */
export function directTarget(share: Pick<IncomingShare, 'target'>, chats: MessengerChat[]): MessengerChat | null {
  if (!share.target) return null;
  return chats.find((c) => c.id === share.target && shareTarget(c)) ?? null;
}

/** What the chooser says is shared: the names of the first files, and how many more. */
export function filesLine(files: SharedFile[], shown = 3): { names: string[]; more: number } {
  return { names: files.slice(0, shown).map((f) => f.name), more: Math.max(0, files.length - shown) };
}

/** The kind of a shared file by its mime, for the icon of the chooser. */
export function kindOf(file: Pick<SharedFile, 'mime' | 'name'>): 'image' | 'video' | 'audio' | 'file' {
  const mime = file.mime ?? '';
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'audio';
  return 'file';
}
