// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The name of a note tag as the notes store it: trimmed and lowercase, so
// "Работа" and "работа" are one tag (platform-spec 9.1). The backend
// stores every name so; the pickers find a tag typed with capitals and send
// the name it is stored under, to the new tag and to the note alike.

export function normalizeTagName(name: string): string {
  return name.trim().toLowerCase();
}

/** The tag the typed name is, whatever its case. */
export function findTag<T extends { name: string }>(tags: readonly T[], name: string): T | undefined {
  const key = normalizeTagName(name);
  return key ? tags.find((tag) => normalizeTagName(tag.name) === key) : undefined;
}

/** What adding the typed name does: nothing when it is empty or on the note
 *  already; else the name to store, and the tag that has it when there is one
 *  (then no new tag is created and its color stays). */
export function tagChoice<T extends { name: string }>(
  typed: string,
  allTags: readonly T[],
  selected: readonly string[],
): { name: string; existing: T | undefined } | null {
  const name = normalizeTagName(typed);
  if (!name || selected.some((n) => normalizeTagName(n) === name)) return null;
  return { name, existing: findTag(allTags, name) };
}
