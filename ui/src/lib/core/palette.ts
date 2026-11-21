// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The one palette a picker offers for what the user colours: a tag, a folder,
 * a workspace, a smart view, a column, a 2FA label (the kit's presets plus
 * teal). The colour is the user's and never tells the kind of an object; the
 * first one is the default. Each passes 4.5:1 as a chip's text in both themes
 * (core/styles/chip-contrast.test.ts). «A colour of your own» next to it is
 * `--hue-wheel`.
 */
export const PALETTE: readonly string[] = ['#8b7bff', '#5ab0f0', '#59c28a', '#e8a33d', '#e66b6b', '#d58ad6', '#2dd4bf'];

/** The names of PALETTE's colours, in its order (for an accessible label). */
export const PALETTE_NAMES = ['violet', 'blue', 'green', 'amber', 'red', 'pink', 'teal'] as const;

export const DEFAULT_COLOR = PALETTE[0];
