// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Veydan Notes capture extension. No persistent content scripts, no network:
// page access happens only after an explicit user action (activeTab),
// then the payload goes to the native host which relays it to the running app.

/* global PROFILE_ID, t */

const HOST = 'veydan_notes';
const MENU = {
  selectionNew: 'veydan-selection-new',
  appendTo: 'veydan-append-to',
  appendLast: 'veydan-append-last',
  appendSearch: 'veydan-append-search',
  appendSep: 'veydan-append-sep',
  bookmark: 'veydan-bookmark',
  article: 'veydan-article',
  screenshot: 'veydan-screenshot',
};
// Child items of the "add to note" submenu carry the note id after this prefix
const APPEND_PREFIX = 'veydan-append:';
const APPEND_MENU_LIMIT = 12;
const MENU_TITLE_MAX = 40;

browser.menus.create({ id: MENU.selectionNew, title: t('menu_selection_new'), contexts: ['selection'] });
browser.menus.create({ id: MENU.appendTo, title: t('menu_append_to'), contexts: ['selection'] });
browser.menus.create({ id: MENU.appendLast, parentId: MENU.appendTo, title: t('menu_append_last'), contexts: ['selection'] });
browser.menus.create({ id: MENU.appendSearch, parentId: MENU.appendTo, title: t('menu_append_search'), contexts: ['selection'] });
browser.menus.create({ id: MENU.appendSep, parentId: MENU.appendTo, type: 'separator', contexts: ['selection'] });

// Native menus have no text input: "find a note" opens the popup in picker mode instead.
let pickRequested = false;

async function openPicker() {
  pickRequested = true;
  try {
    await browser.browserAction.openPopup();
  } catch {
    pickRequested = false;
    notify(t('notify_title'), t('notify_open_popup'));
  }
}
browser.menus.create({ id: MENU.bookmark, title: t('menu_bookmark'), contexts: ['page'] });
browser.menus.create({ id: MENU.article, title: t('menu_article'), contexts: ['page'] });
browser.menus.create({ id: MENU.screenshot, title: t('menu_screenshot'), contexts: ['page'] });

// ── "Add to note" submenu: rebuilt with this page's notes each time the menu opens ──
let appendItemIds = [];

function truncate(s) {
  const clean = (s || t('untitled')).replace(/\s+/g, ' ').trim();
  return clean.length > MENU_TITLE_MAX ? clean.slice(0, MENU_TITLE_MAX - 1) + '…' : clean;
}

async function rebuildAppendMenu(tab) {
  const res = await send({ kind: 'list_notes', url: tab.url || '' });
  await Promise.all(appendItemIds.map((id) => browser.menus.remove(id).catch(() => {})));
  appendItemIds = [];
  const notes = (res && res.ok && res.notes) || [];
  for (const n of notes.slice(0, APPEND_MENU_LIMIT)) {
    const id = APPEND_PREFIX + n.id;
    browser.menus.create({ id, parentId: MENU.appendTo, title: truncate(n.title), contexts: ['selection'] });
    appendItemIds.push(id);
  }
  // Separator only makes sense when there is a list below it
  browser.menus.update(MENU.appendSep, { visible: appendItemIds.length > 0 });
}

browser.menus.onShown.addListener(async (info, tab) => {
  if (!tab || !info.contexts.includes('selection')) return;
  await rebuildAppendMenu(tab);
  browser.menus.refresh();
});

async function readSelection(tabId) {
  try {
    const [res] = await browser.tabs.executeScript(tabId, {
      code: `(() => {
        const s = window.getSelection();
        if (!s || s.rangeCount === 0) return { text: '', html: '' };
        const box = document.createElement('div');
        for (let i = 0; i < s.rangeCount; i++) box.appendChild(s.getRangeAt(i).cloneContents());
        return { text: s.toString(), html: box.innerHTML };
      })()`,
    });
    return res || { text: '', html: '' };
  } catch {
    // Privileged pages (about:, addons) refuse scripts; fall back to a bookmark
    return { text: '', html: '' };
  }
}

async function readArticle(tabId) {
  try {
    const [res] = await browser.tabs.executeScript(tabId, { file: 'article.js' });
    return res && res.markdown ? res : null;
  } catch {
    return null;
  }
}

async function takeScreenshot(tab) {
  const dataUrl = await browser.tabs.captureVisibleTab(tab.windowId, { format: 'png' });
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  return { src: '', name: `screenshot-${stamp}.png`, data_b64: dataUrl.split(',')[1] || '' };
}

function notify(title, message) {
  browser.notifications.create({ type: 'basic', title, message });
}

const FEEDBACK_MS = 2200;

/** Short badge on the toolbar icon: green check on success, red cross on failure. */
function flashBadge(tabId, ok) {
  browser.browserAction.setBadgeBackgroundColor({ tabId, color: ok ? '#2ea043' : '#e5484d' });
  browser.browserAction.setBadgeText({ tabId, text: ok ? '✓' : '!' });
  setTimeout(() => browser.browserAction.setBadgeText({ tabId, text: '' }).catch(() => {}), FEEDBACK_MS);
}

/** In-page toast (shadow DOM, isolated from page CSS); resolves false on privileged pages. */
async function showToast(tabId, title, subtitle, ok) {
  const args = JSON.stringify({ title, subtitle, ok, ms: FEEDBACK_MS });
  try {
    await browser.tabs.executeScript(tabId, {
      code: `(() => {
        const { title, subtitle, ok, ms } = ${args};
        document.getElementById('veydan-notes-toast')?.remove();
        const host = document.createElement('div');
        host.id = 'veydan-notes-toast';
        const root = host.attachShadow({ mode: 'closed' });
        root.innerHTML = \`
          <style>
            .t { position: fixed; right: 20px; bottom: 20px; z-index: 2147483647; display: flex; align-items: center; gap: 10px;
                 padding: 10px 14px; border-radius: 10px; background: #1c1c22; color: #f2f2f5; font: 13px/1.3 system-ui, sans-serif;
                 box-shadow: 0 8px 28px rgba(0,0,0,.35); animation: in .22s ease-out, out .3s ease-in \${ms - 300}ms forwards; max-width: 320px; }
            .i { width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; font-size: 13px; flex-shrink: 0;
                 background: \${ok ? '#2ea043' : '#e5484d'}; animation: pop .35s cubic-bezier(.2,1.6,.4,1) .1s both; }
            .s { opacity: .7; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
            @keyframes in { from { transform: translateY(12px); opacity: 0 } to { transform: none; opacity: 1 } }
            @keyframes out { to { transform: translateY(8px); opacity: 0 } }
            @keyframes pop { from { transform: scale(.4) } to { transform: scale(1) } }
          </style>
          <div class="t"><div class="i">\${ok ? '✓' : '!'}</div><div><div></div><div class="s"></div></div></div>\`;
        const [t, s] = root.querySelectorAll('.t > div:last-child > div');
        t.textContent = title; s.textContent = subtitle; if (!subtitle) s.remove();
        document.documentElement.appendChild(host);
        setTimeout(() => host.remove(), ms);
      })()`,
    });
    return true;
  } catch {
    return false;
  }
}

/** Success/failure feedback: badge always, toast when the page allows it, system notification otherwise. */
async function feedback(tab, ok, title, subtitle) {
  flashBadge(tab.id, ok);
  if (!(await showToast(tab.id, title, subtitle, ok))) notify(title, subtitle);
}

async function send(req) {
  try {
    return await browser.runtime.sendNativeMessage(HOST, { profile_id: PROFILE_ID, ...req });
  } catch {
    return { ok: false, error: t('notify_host') };
  }
}

async function capture(kind, tab, extra = {}) {
  const res = await send({ kind, url: tab.url || '', title: tab.title || '', ...extra });
  if (res && res.ok) await feedback(tab, true, t('notify_saved'), res.title || '');
  else await feedback(tab, false, t('notify_title'), (res && res.error) || t('notify_failed'));
  return res;
}

/** `extra` carries per-call fields such as `note_id` for appends. */
async function captureFromTab(kind, tab, extra = {}) {
  if (kind === 'article') {
    const article = await readArticle(tab.id);
    if (!article) return capture('bookmark', tab);
    return capture('article', tab, { title: article.title || tab.title, markdown: article.markdown, assets: article.images });
  }
  if (kind === 'screenshot') {
    try {
      return await capture('screenshot', tab, { assets: [await takeScreenshot(tab)] });
    } catch {
      await feedback(tab, false, t('notify_title'), t('notify_no_screenshot'));
      return { ok: false };
    }
  }
  if (kind === 'bookmark') return capture('bookmark', tab);
  const selection = await readSelection(tab.id);
  // Appending to a chosen note works without a selection (the page link is added)
  if (!selection.text.trim() && !extra.note_id) kind = 'bookmark';
  return capture(kind, tab, { selection_text: selection.text, selection_html: selection.html, ...extra });
}

const MENU_KIND = {
  [MENU.selectionNew]: 'selection_new',
  [MENU.appendLast]: 'selection_append',
  [MENU.bookmark]: 'bookmark',
  [MENU.article]: 'article',
  [MENU.screenshot]: 'screenshot',
};

browser.menus.onClicked.addListener((info, tab) => {
  if (!tab) return;
  const id = String(info.menuItemId);
  if (id.startsWith(APPEND_PREFIX)) {
    captureFromTab('selection_append', tab, { note_id: id.slice(APPEND_PREFIX.length) });
    return;
  }
  if (id === MENU.appendSearch) {
    openPicker();
    return;
  }
  const kind = MENU_KIND[id];
  if (kind) captureFromTab(kind, tab);
});

async function activeTab() {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  return tab;
}

browser.commands.onCommand.addListener(async (command) => {
  const tab = await activeTab();
  if (!tab) return;
  if (command === 'save-selection') captureFromTab('selection_new', tab);
  if (command === 'save-article') captureFromTab('article', tab);
});

// Popup -> background
browser.runtime.onMessage.addListener(async (msg) => {
  const tab = await activeTab();
  if (!tab) return { ok: false, error: 'No active tab' };
  switch (msg.type) {
    case 'list': return send({ kind: 'list_notes', url: tab.url || '' });
    // Popup asks once on open whether it was launched from the "find a note" menu item
    case 'pick_requested': {
      const requested = pickRequested;
      pickRequested = false;
      return { ok: true, requested };
    }
    case 'search': return send({ kind: 'search_notes', query: msg.query || '' });
    case 'open': return send({ kind: 'open_note', note_id: msg.noteId });
    case 'capture': return captureFromTab(msg.kind, tab, msg.noteId ? { note_id: msg.noteId } : {});
    default: return { ok: false, error: 'Unknown message' };
  }
});
