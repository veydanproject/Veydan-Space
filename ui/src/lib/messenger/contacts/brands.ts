// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The marks of the platforms a social link can be of (Icon.svelte,
// `brand-<id>`: simple-icons, CC0).

const BRANDS = new Set([
  'telegram', 'instagram', 'tiktok', 'x', 'youtube', 'vk', 'facebook', 'linkedin', 'github', 'whatsapp',
  'discord', 'twitch', 'mastodon', 'bluesky', 'threads', 'reddit',
]);

/** The icon of a platform; a plain globe for "other" and for any platform newer than this UI. */
export function brandIcon(platform: string): string {
  return BRANDS.has(platform) ? `brand-${platform}` : 'globe';
}
