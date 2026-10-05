// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Lines the Tor install block shows, kept apart from the markup so they can be tested.

/** Whole megabytes, as the Camoufox card shows a download. */
export function formatMb(bytes: number): string {
  return (bytes / 1024 / 1024).toFixed(0) + ' MB';
}

/** A bar's width: whole percent, kept inside 0..100 whatever the backend sent. */
export function clampPercent(percent: number | null | undefined): number {
  if (percent == null || !Number.isFinite(percent)) return 0;
  return Math.min(100, Math.max(0, Math.round(percent)));
}

/** "downloaded / total · percent", or just the percent while the size is unknown. */
export function progressLine(p: { downloaded: number; total: number; percent: number }): string {
  const percent = clampPercent(p.percent);
  if (!p.total) return `${percent}%`;
  return `${formatMb(p.downloaded)} / ${formatMb(p.total)} · ${percent}%`;
}

/** Versions on one line: "14.5.8 · tor 0.4.8.17", whichever of the two are known. */
export function versionLine(version: string | null, torVersion: string | null): string {
  return [version, torVersion ? `tor ${torVersion}` : null].filter(Boolean).join(' · ');
}

/** The error of a cancelled download is not shown: the user asked for it. */
export function isCancelled(message: string): boolean {
  return message.includes('cancelled');
}
