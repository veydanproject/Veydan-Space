// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * `1.4 MB`, binary units, one decimal below 10. With `lang` (a BCP-47 tag)
 * the number and the unit are that language's: `5,2 МБ`.
 */
export function bytes(n: number, lang?: string): string {
  if (!Number.isFinite(n) || n <= 0) return lang ? `0${NBSP}${unitOf('byte', 'narrow', lang)}` : '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  let digits = v < 10 && i > 0 ? 1 : 0;
  // Rounded as shown first: just under 1024 of a unit is 1,0 of the next one, never 1 024 of it.
  const shown = digits ? Math.round(v * 10) / 10 : Math.round(v);
  if (shown >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
    digits = v < 10 ? 1 : 0;
  }
  if (!lang) return `${digits ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
  const number = new Intl.NumberFormat(lang, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(v);
  return `${number}${NBSP}${unitOf(BYTE_UNITS[i], 'narrow', lang)}`;
}

const BYTE_UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte'] as const;
/** Between a number and its unit: the two never part at the end of a line. */
const NBSP = '\u00a0';

/** The name of a unit as a language writes it after a number. */
function unitOf(unit: string, display: 'narrow' | 'short', lang: string): string {
  const parts = new Intl.NumberFormat(lang, { style: 'unit', unit, unitDisplay: display }).formatToParts(5);
  return parts.find((p) => p.type === 'unit')?.value ?? unit;
}

/** A speed: `5,2 МБ/с`, `5.2 MB/s`, `5,2 MB/s`; '' while nothing moves. */
export function rate(bps: number, lang = 'en'): string {
  if (!Number.isFinite(bps) || bps <= 0) return '';
  return `${bytes(bps, lang)}${perSecond(lang)}`;
}

/**
 * What makes a size a speed, as the language writes it after `MB`: `/s`,
 * also where its own short second is a word (`Sek.` in German). Russian
 * keeps its own `с`: CLDR writes its `МБ/c` with a Latin letter.
 */
function perSecond(lang: string): string {
  if (lang.split('-')[0].toLowerCase() !== 'ru') {
    try {
      const speed = unitOf('megabyte-per-second', 'narrow', lang);
      const size = unitOf('megabyte', 'narrow', lang);
      if (speed.startsWith(size) && speed.length > size.length) return speed.slice(size.length);
    } catch {
      // No compound units here: the unit of a second follows.
    }
  }
  return `/${unitOf('second', 'narrow', lang)}`;
}

/** Time left, roughly: `~40 с`, `~2 мин`, `~1 ч`; '' when unknown. */
export function eta(secs: number | null | undefined, lang = 'en'): string {
  if (secs == null || !Number.isFinite(secs) || secs <= 0) return '';
  const [unit, v] = secs < 60 ? ['second', Math.max(5, Math.round(secs / 5) * 5)] as const
    : secs < 3600 ? ['minute', Math.max(1, Math.round(secs / 60))] as const
    : ['hour', Math.round(secs / 3600)] as const;
  // 58 s rounds to 60: one minute, not "60 s".
  if (unit === 'second' && v >= 60) return `~${duration(1, 'minute', lang)}`;
  if (unit === 'minute' && v >= 60) return `~${duration(1, 'hour', lang)}`;
  return `~${duration(v, unit, lang)}`;
}

/** `12 с`, `2 min`: a whole number of a unit of time. */
export function duration(n: number, unit: 'second' | 'minute' | 'hour', lang = 'en'): string {
  return new Intl.NumberFormat(lang, { style: 'unit', unit, unitDisplay: 'short', maximumFractionDigits: 0 }).format(n).replace(/ /g, NBSP);
}

export function percent(done: number, total: number): number {
  if (!total) return 0;
  return Math.max(0, Math.min(100, Math.round((done / total) * 100)));
}

const ARCHIVES = new Set(['zip', 'rar', '7z', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst']);
const CODE = new Set([
  'js', 'mjs', 'cjs', 'ts', 'tsx', 'jsx', 'svelte', 'vue', 'css', 'scss', 'html', 'htm', 'rs', 'go', 'py', 'rb', 'php',
  'java', 'kt', 'swift', 'c', 'h', 'cpp', 'hpp', 'cs', 'dart', 'sh', 'bash', 'zsh', 'ps1', 'sql', 'toml', 'yaml', 'yml', 'xml',
]);
const TABLES = new Set(['csv', 'tsv', 'xls', 'xlsx', 'ods', 'numbers']);
const DOCUMENTS = new Set(['pdf', 'doc', 'docx', 'odt', 'rtf', 'txt', 'md', 'pages', 'epub']);

/** Icon of a file by what it is: its type first, its extension when the type says little. */
export function fileIcon(name: string, mime: string): string {
  const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
  if (mime.startsWith('image/')) return 'image';
  if (mime.startsWith('video/')) return 'video';
  if (mime.startsWith('audio/')) return 'mic';
  if (ext === 'json' || mime === 'application/json') return 'file-json';
  if (ARCHIVES.has(ext) || /zip|compressed|x-tar|x-7z/.test(mime)) return 'archive';
  if (TABLES.has(ext) || /spreadsheet|csv/.test(mime)) return 'table-2';
  if (CODE.has(ext)) return 'file-code';
  if (DOCUMENTS.has(ext) || mime === 'application/pdf' || /word|document|text\//.test(mime)) return 'file-text';
  return 'file';
}

/**
 * Filters of the save dialog for a file called `name`: its own extension
 * first, then every file. Windows and macOS drop the extension of a name
 * that no filter says; none for a name without one. On macOS rfd merges
 * the lists into one, where `*` matches nothing: the own extension stays
 * first, so the panel adds it to a name typed without one.
 */
export function saveFilters(name: string, allFiles: string): { name: string; extensions: string[] }[] {
  const ext = /\.([a-z0-9]{1,12})$/i.exec(name)?.[1];
  if (!ext) return [];
  return [
    { name: ext.toUpperCase(), extensions: [ext.toLowerCase()] },
    { name: allFiles, extensions: ['*'] },
  ];
}
