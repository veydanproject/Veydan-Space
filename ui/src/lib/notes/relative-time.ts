// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The time of a note's row in the list, told from now: «только что»,
// «5 мин назад», «3 ч назад», «2 д назад». The list of the computer and the
// phone's say it alike (design kit, notes-list).

type Translate = (key: 'notes_time_just_now' | 'notes_time_m_ago' | 'notes_time_h_ago' | 'notes_time_d_ago', params?: Record<string, string>) => string;

export function relativeTime(iso: string, t: Translate, now = Date.now()): string {
  const m = Math.floor((now - new Date(iso).getTime()) / 60000);
  if (m < 1) return t('notes_time_just_now');
  if (m < 60) return t('notes_time_m_ago', { m: String(m) });
  const h = Math.floor(m / 60);
  if (h < 24) return t('notes_time_h_ago', { h: String(h) });
  return t('notes_time_d_ago', { d: String(Math.floor(h / 24)) });
}
