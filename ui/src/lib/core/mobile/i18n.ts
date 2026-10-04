// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Mobile strings layered over the desktop dictionary: mobile keys win,
// everything else falls through to $lib/core/i18n. The phone keys of the product's
// modules come in through the virtual module, like the desktop ones.

import { derived } from 'svelte/store';
import { locale, t as desktopT, type Locale, type TranslationKey } from '$lib/core/i18n';
import { translations as moduleTranslations } from 'virtual:veydan-modules/i18n';

const mobile = {
  en: {
    ...moduleTranslations.mobile.en,
    app_launcher: 'Apps',
    notes_sub: 'Markdown notes',
    notes_pinned: 'Pinned',
    notes_saving: 'Saving…',
    notes_item_more: 'More',
    notes_edit: 'Edit',
    notes_preview: 'Preview',
    notes_sync_now: 'Sync now',
    notes_sync_pending: 'Local changes not pushed yet',
    notes_sync_uptodate: 'Up to date',
    notes_sync_untracked: 'Not in the vault yet',
    notes_sync_off: 'Sync is off',
    common_save: 'Save',
    common_saved: 'Saved',
    sync_interval_sec: '{n} s',
    sync_interval_min: '{n} min',
    totp_sub: 'One-time codes',
    common_back: 'Back',
    app_settings: 'Settings',
    apps_title: 'Apps',
    apps_sub: 'Everything in one place',
    settings_appearance: 'Appearance',
    settings_theme: 'Theme',
    settings_theme_dark: 'Dark',
    settings_theme_light: 'Light',
    settings_language: 'Language',
    settings_about: 'About',
    settings_version: 'Version',
    nav_home: 'Home',
    hub_sub: 'Choose an app',
    home_morning: 'Good morning',
    home_day: 'Good afternoon',
    home_evening: 'Good evening',
    home_night: 'Good night',
    home_search: 'Search in Veydan',
    notes_quick_menu: 'Quick actions',
    settings_main: 'General',
    settings_security: 'Security',
    settings_data: 'Data',
    settings_default_app: 'Default app',
    settings_default_home: 'Home',
    settings_start_screen: 'Start screen',
    settings_sync_connected: 'Connected',
    settings_sync_disconnected: 'Not connected',
    totp_add_account: 'Add account',
    update_check: 'Check for updates',
    update_checking: 'Checking…',
    update_up_to_date: 'Latest version',
    update_available: 'Version {version} is available',
    update_available_hint: 'Download the APK on the release page and install it over this version.',
    update_open: 'Download',
    common_clear: 'Clear',
  },
  ru: {
    ...moduleTranslations.mobile.ru,
    app_launcher: 'Приложения',
    notes_sub: 'Заметки в Markdown',
    notes_pinned: 'Закреплённые',
    notes_saving: 'Сохранение…',
    notes_item_more: 'Ещё',
    notes_edit: 'Текст',
    notes_preview: 'Просмотр',
    notes_sync_now: 'Синхронизировать',
    notes_sync_pending: 'Есть локальные изменения, ещё не отправлены',
    notes_sync_uptodate: 'Актуально',
    notes_sync_untracked: 'Ещё не в сейфе',
    notes_sync_off: 'Синхронизация выключена',
    common_save: 'Сохранить',
    common_saved: 'Сохранено',
    sync_interval_sec: '{n} с',
    sync_interval_min: '{n} мин',
    totp_sub: 'Одноразовые коды',
    common_back: 'Назад',
    app_settings: 'Настройки',
    apps_title: 'Приложения',
    apps_sub: 'Всё в одном месте',
    settings_appearance: 'Оформление',
    settings_theme: 'Тема',
    settings_theme_dark: 'Тёмная',
    settings_theme_light: 'Светлая',
    settings_language: 'Язык',
    settings_about: 'О приложении',
    settings_version: 'Версия',
    nav_home: 'Главная',
    hub_sub: 'Выберите приложение',
    home_morning: 'Доброе утро',
    home_day: 'Здравствуйте',
    home_evening: 'Добрый вечер',
    home_night: 'Доброй ночи',
    home_search: 'Поиск в Veydan...',
    notes_quick_menu: 'Быстрое меню',
    settings_main: 'Основное',
    settings_security: 'Безопасность',
    settings_data: 'Данные',
    settings_default_app: 'Приложение по умолчанию',
    settings_default_home: 'Главная',
    settings_start_screen: 'Стартовый экран',
    settings_sync_connected: 'Подключено',
    settings_sync_disconnected: 'Не подключено',
    totp_add_account: 'Добавить аккаунт',
    update_check: 'Проверить обновления',
    update_checking: 'Проверка…',
    update_up_to_date: 'Последняя версия',
    update_available: 'Доступна версия {version}',
    update_available_hint: 'Скачайте APK на странице выпуска и установите поверх этой версии.',
    update_open: 'Скачать',
    common_clear: 'Очистить',
  },
} as const satisfies Record<Locale, Record<string, string>>;

export type MobileKey = keyof typeof mobile.en;
export type Key = MobileKey | TranslationKey;

export { locale };
export type { Locale };

function interpolate(text: string, vars?: Record<string, string>): string {
  if (!vars) return text;
  for (const [k, v] of Object.entries(vars)) text = text.split(`{${k}}`).join(v);
  return text;
}

export const t = derived([locale, desktopT], ([$locale, $t]) => {
  return (key: Key, vars?: Record<string, string>): string => {
    if (key in mobile.en) {
      const dict = mobile[$locale] as Record<string, string>;
      return interpolate(dict[key] ?? mobile.en[key as MobileKey], vars);
    }
    return $t(key as TranslationKey, vars);
  };
});

/** The merged phone layer as `t()` reads it. */
export { mobile as mobileDictionary };
