// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/* global t, applyI18n */

applyI18n(document);

const SCOPE_LABEL = { url: t('scope_url'), domain: t('scope_domain'), profile: t('scope_profile'), all: '' };
const list = document.getElementById('notes');
const picker = document.getElementById('picker');
const queryEl = document.getElementById('query');
const results = document.getElementById('results');

function message(container, cls, text) {
  container.textContent = '';
  const el = document.createElement('div');
  el.className = cls;
  el.textContent = text;
  container.appendChild(el);
}

/** Render note buttons into `container`; `onpick(note)` handles the click. */
function renderNotes(container, res, emptyText, onpick) {
  if (!res || !res.ok) {
    message(container, 'error', (res && res.error) || t('not_running'));
    return;
  }
  const notes = res.notes || [];
  if (notes.length === 0) {
    message(container, 'empty', emptyText);
    return;
  }
  container.textContent = '';
  for (const n of notes) {
    const btn = document.createElement('button');
    btn.className = 'note';
    const title = document.createElement('span');
    title.className = 'title';
    title.textContent = n.title || t('untitled');
    const scope = document.createElement('span');
    scope.className = 'scope';
    scope.textContent = SCOPE_LABEL[n.scope] ?? n.scope;
    btn.append(title, scope);
    btn.addEventListener('click', () => onpick(n));
    container.appendChild(btn);
  }
}

async function openNote(n) {
  await browser.runtime.sendMessage({ type: 'open', noteId: n.id });
  window.close();
}

async function appendTo(n) {
  await browser.runtime.sendMessage({ type: 'capture', kind: 'selection_append', noteId: n.id });
  window.close();
}

async function refresh() {
  const res = await browser.runtime.sendMessage({ type: 'list' });
  renderNotes(list, res, t('no_notes'), openNote);
  // Page notes double as the default pick list until the user types
  renderNotes(results, res, t('type_to_search'), appendTo);
}

// ── Pick a target note for "Add to note…" ────────────────────────────────────
let searchTimer = null;

async function search() {
  const q = queryEl.value.trim();
  if (!q) {
    renderNotes(results, await browser.runtime.sendMessage({ type: 'list' }), t('type_to_search'), appendTo);
    return;
  }
  renderNotes(results, await browser.runtime.sendMessage({ type: 'search', query: q }), t('nothing_found'), appendTo);
}

function showPicker() {
  picker.classList.remove('hidden');
  queryEl.focus();
}

document.getElementById('pick').addEventListener('click', () => {
  if (picker.classList.contains('hidden')) showPicker();
  else picker.classList.add('hidden');
});

// Opened from the context menu "find a note…": start in picker mode
browser.runtime.sendMessage({ type: 'pick_requested' }).then((res) => {
  if (res && res.requested) showPicker();
});

queryEl.addEventListener('input', () => {
  if (searchTimer) clearTimeout(searchTimer);
  searchTimer = setTimeout(search, 200);
});

queryEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') results.querySelector('button')?.click();
  if (e.key === 'Escape') picker.classList.add('hidden');
});

for (const btn of document.querySelectorAll('.actions button[data-kind]')) {
  btn.addEventListener('click', async () => {
    btn.disabled = true;
    await browser.runtime.sendMessage({ type: 'capture', kind: btn.dataset.kind });
    window.close();
  });
}

refresh();
