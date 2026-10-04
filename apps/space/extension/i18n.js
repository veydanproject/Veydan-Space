// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Extension strings follow the app language (LOCALE from config.js), not the browser's.

/* global LOCALE */

const STRINGS = {
  en: {
    save_selection: 'Save selection',
    save_selection_hint: 'New note from the selected text, or a bookmark of the page when nothing is selected',
    add_to_note: 'Add to note…',
    add_to_note_hint: 'Append the selected text to an existing note',
    save_article: 'Save article',
    save_article_hint: 'Readable article body converted to Markdown, images attached',
    save_screenshot: 'Save screenshot',
    save_screenshot_hint: 'Screenshot of the visible page as an attachment',
    search_placeholder: 'Search notes by title or text…',
    pick_hint: 'Pick a note to append the selection to',
    notes_for_page: 'Notes for this page',
    loading: 'Loading…',
    no_notes: 'No notes yet',
    type_to_search: 'Type to search all notes',
    nothing_found: 'Nothing found',
    not_running: 'Veydan is not running, or Notes are switched off in it',
    untitled: 'Untitled',
    scope_url: 'this page',
    scope_domain: 'this site',
    scope_profile: 'this profile',
    how_it_works: 'How it works',
    help_1: 'Select text on a page, then click a button here, use the right-click menu, or press Alt+Shift+N (Alt+Shift+M for the article).',
    help_2: 'Every capture is saved in the Veydan app and linked to this page, this site and this browser profile, so it shows up in the list above whenever you come back.',
    help_3: 'Veydan must be running. Nothing is sent over the network.',
    menu_selection_new: 'Save selection to new note',
    menu_append_to: 'Add selection to note',
    menu_append_last: 'Last note',
    menu_append_search: 'Find a note…',
    menu_bookmark: 'Save page to Veydan notes',
    menu_article: 'Save article as Markdown',
    menu_screenshot: 'Save screenshot to Veydan notes',
    notify_saved: 'Saved to Veydan',
    notify_title: 'Veydan Notes',
    notify_failed: 'Capture failed',
    notify_host: 'Veydan is not running, Notes are switched off in it, or the capture host is not registered.',
    notify_no_screenshot: 'Screenshot is not available on this page.',
    notify_open_popup: 'Could not open the note picker. Click the Veydan icon in the toolbar and use "Add to note…".',
  },
  ru: {
    save_selection: 'Сохранить выделение',
    save_selection_hint: 'Новая заметка из выделенного текста; без выделения — закладка на страницу',
    add_to_note: 'Добавить в заметку…',
    add_to_note_hint: 'Дописать выделенный текст в существующую заметку',
    save_article: 'Сохранить статью',
    save_article_hint: 'Текст статьи в Markdown, картинки во вложениях',
    save_screenshot: 'Сохранить скриншот',
    save_screenshot_hint: 'Скриншот видимой части страницы во вложение',
    search_placeholder: 'Поиск заметок по названию или тексту…',
    pick_hint: 'Выберите заметку, в которую добавить выделение',
    notes_for_page: 'Заметки этой страницы',
    loading: 'Загрузка…',
    no_notes: 'Заметок пока нет',
    type_to_search: 'Введите текст для поиска по всем заметкам',
    nothing_found: 'Ничего не найдено',
    not_running: 'Veydan не запущен или в нём выключены заметки',
    untitled: 'Без названия',
    scope_url: 'эта страница',
    scope_domain: 'этот сайт',
    scope_profile: 'этот профиль',
    how_it_works: 'Как это работает',
    help_1: 'Выделите текст на странице и нажмите кнопку здесь, используйте правую кнопку мыши или Alt+Shift+N (Alt+Shift+M — для статьи).',
    help_2: 'Каждое сохранение попадает в приложение Veydan и привязывается к этой странице, сайту и профилю браузера — заметка появится в списке выше, когда вы вернётесь.',
    help_3: 'Veydan должен быть запущен. Ничего не отправляется в сеть.',
    menu_selection_new: 'Сохранить выделение в новую заметку',
    menu_append_to: 'Добавить выделение в заметку',
    menu_append_last: 'Последняя заметка',
    menu_append_search: 'Найти заметку…',
    menu_bookmark: 'Сохранить страницу в заметки Veydan',
    menu_article: 'Сохранить статью как Markdown',
    menu_screenshot: 'Сохранить скриншот в заметки Veydan',
    notify_saved: 'Сохранено в Veydan',
    notify_title: 'Veydan Notes',
    notify_failed: 'Не удалось сохранить',
    notify_host: 'Veydan не запущен, в нём выключены заметки или хост захвата не зарегистрирован.',
    notify_no_screenshot: 'Скриншот недоступен на этой странице.',
    notify_open_popup: 'Не удалось открыть выбор заметки. Нажмите иконку Veydan в панели и «Добавить в заметку…».',
  },
};

function t(key) {
  const table = STRINGS[typeof LOCALE === 'string' ? LOCALE : 'en'] || STRINGS.en;
  return table[key] ?? STRINGS.en[key] ?? key;
}

/** Fill `data-i18n` (text) and `data-i18n-title` / `data-i18n-placeholder` attributes. */
function applyI18n(root) {
  for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n);
  for (const el of root.querySelectorAll('[data-i18n-title]')) el.title = t(el.dataset.i18nTitle);
  for (const el of root.querySelectorAll('[data-i18n-placeholder]')) el.placeholder = t(el.dataset.i18nPlaceholder);
}
