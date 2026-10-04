// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Placeholders rendered by `templates.rs` when a note is created from a template. */
export const TEMPLATE_PLACEHOLDERS = [
  'date', 'time', 'datetime', 'title',
  'workspace', 'profile', 'url', 'domain',
  'proxy', 'proxy_type', 'proxy_host', 'proxy_port', 'proxy_region',
  'ssh', 'ssh_host', 'ssh_port', 'ssh_user',
  'totp',
  'password',
] as const;

export type TemplatePlaceholder = (typeof TEMPLATE_PLACEHOLDERS)[number];

/** Placeholders containing `query`, exact prefix matches first. */
export function searchPlaceholders(query: string): TemplatePlaceholder[] {
  const q = query.trim().toLowerCase();
  return TEMPLATE_PLACEHOLDERS
    .filter((p) => p.includes(q))
    .sort((a, b) => Number(!a.startsWith(q)) - Number(!b.startsWith(q)) || a.localeCompare(b));
}

export function placeholderMarkup(name: string): string {
  return `{{${name}}}`;
}
