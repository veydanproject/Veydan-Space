// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Module-local strings; the host passes the locale.

export type MediaLocale = 'en' | 'ru';

const en = {
  audio: 'Audio',
  video: 'Video',
  record: 'Record',
  camera: 'Camera',
  attach: 'Attach',
  quality: 'Quality',
  resolution: 'Resolution',
  camera_front: 'Front',
  camera_back: 'Back',
  recordings: 'Recordings ({n})',
  play: 'Play',
  stop_play: 'Stop',
  actions: 'Recording',
  insert: 'Insert',
  save: 'Save to device',
  delete: 'Delete',
  not_downloaded: 'Not downloaded',
  opening: 'Starting…',
  start: 'Start recording',
  stop: 'Stop',
  switch_camera: 'Switch camera',
  use: 'Use',
  retake: 'Retake',
  cancel: 'Cancel',
  close: 'Close',
  err_denied: 'Access to the microphone or camera was denied.',
  err_no_device: 'No microphone or camera found.',
  err_busy: 'The device is in use by another app.',
  err_unsupported: 'Recording is not supported here.',
};

const ru: typeof en = {
  audio: 'Аудио',
  video: 'Видео',
  record: 'Записать',
  camera: 'Камера',
  attach: 'Прикрепить',
  quality: 'Качество',
  resolution: 'Разрешение',
  camera_front: 'Фронтальная',
  camera_back: 'Основная',
  recordings: 'Записи ({n})',
  play: 'Воспроизвести',
  stop_play: 'Остановить',
  actions: 'Запись',
  insert: 'Вставить',
  save: 'Сохранить на устройство',
  delete: 'Удалить',
  not_downloaded: 'Не скачано',
  opening: 'Запуск…',
  start: 'Начать запись',
  stop: 'Стоп',
  switch_camera: 'Переключить камеру',
  use: 'Использовать',
  retake: 'Переснять',
  cancel: 'Отмена',
  close: 'Закрыть',
  err_denied: 'Доступ к микрофону или камере запрещён.',
  err_no_device: 'Микрофон или камера не найдены.',
  err_busy: 'Устройство занято другим приложением.',
  err_unsupported: 'Запись здесь не поддерживается.',
};

export type MediaKey = keyof typeof en;

const dict: Record<MediaLocale, typeof en> = { en, ru };

export function mediaT(locale: MediaLocale) {
  return (key: MediaKey, vars?: Record<string, string>): string => {
    let text: string = dict[locale][key] ?? en[key];
    if (vars) for (const [k, v] of Object.entries(vars)) text = text.replace(`{${k}}`, v);
    return text;
  };
}

/** mm:ss for a duration. */
export function fmtDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
}
