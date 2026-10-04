// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** `1.4 MB`, binary units, one decimal below 10. */
export function bytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
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
