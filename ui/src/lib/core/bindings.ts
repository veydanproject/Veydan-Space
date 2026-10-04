// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Note binding kinds. A binding is a `kind:value` string stored on the note.
 * Entity kinds point at a Veydan object by id; value kinds carry the value (url, domain).
 * Mirrors `crates/notes/src/binding.rs`.
 */

export const BINDING_KINDS = ['workspace', 'profile', 'proxy', 'ssh', 'totp', 'password', 'note', 'url', 'domain'] as const;
export type BindingKind = (typeof BINDING_KINDS)[number];

export const ENTITY_KINDS = ['workspace', 'profile', 'proxy', 'ssh', 'totp', 'password'] as const;
export type EntityKind = (typeof ENTITY_KINDS)[number];

export interface Binding {
  kind: BindingKind;
  value: string;
}

export function binding(kind: BindingKind, value: string): string {
  return `${kind}:${value}`;
}

/** Split `kind:value`; null for unknown prefixes. */
export function parseBinding(s: string): Binding | null {
  const i = s.indexOf(':');
  if (i < 0) return null;
  const kind = s.slice(0, i) as BindingKind;
  if (!BINDING_KINDS.includes(kind)) return null;
  return { kind, value: s.slice(i + 1) };
}

export function isEntityKind(kind: string): kind is EntityKind {
  return (ENTITY_KINDS as readonly string[]).includes(kind);
}

/** `kind:id` for a Veydan entity kind with a non-empty id. */
export function isEntityBinding(s: string): boolean {
  const b = parseBinding(s);
  return !!b && isEntityKind(b.kind) && b.value.length > 0;
}

/** Workspace/profile scope; notes without one are global. */
export function isScopeBinding(s: string): boolean {
  const b = parseBinding(s);
  return b?.kind === 'workspace' || b?.kind === 'profile';
}

/** Value of the first binding of `kind`, if any. */
export function bindingValue(list: string[], kind: BindingKind): string | undefined {
  return list.map(parseBinding).find((b) => b?.kind === kind)?.value;
}

export function hasBindingKind(list: string[], kind: BindingKind): boolean {
  return bindingValue(list, kind) !== undefined;
}
