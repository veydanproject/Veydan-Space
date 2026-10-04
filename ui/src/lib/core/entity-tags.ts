// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { parseBinding } from '$lib/core/bindings';

/** `kind:value` bindings share the tags array with free labels. */
export function isSystemTag(tag: string): boolean {
  const parsed = parseBinding(tag);
  return !!parsed && parsed.value.length > 0;
}

/** Free labels. These are the ones that can show up under a note tag. */
export function userLabels(tags: string[]): string[] {
  return tags.filter((tag) => !isSystemTag(tag));
}

/** Profile, workspace and other `kind:value` bindings. */
export function systemTags(tags: string[]): string[] {
  return tags.filter(isSystemTag);
}

/** Locked context bindings, chosen bindings, and labels, without duplicates. */
export function mergeTags(locked: string[], bindings: string[], labels: string[]): string[] {
  const out: string[] = [];
  const push = (tag: string) => {
    const value = tag.trim();
    if (!value || out.includes(value)) return;
    out.push(value);
  };
  for (const tag of locked) if (isSystemTag(tag)) push(tag);
  for (const tag of bindings) if (isSystemTag(tag)) push(tag);
  for (const tag of labels) if (!isSystemTag(tag)) push(tag);
  return out;
}