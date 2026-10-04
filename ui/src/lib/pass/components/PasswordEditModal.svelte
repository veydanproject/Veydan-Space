<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { api } from '$lib/pass/api';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { generatePassword, pwSettings } from '$lib/pass/password-gen';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory } from '$lib/core/directory';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { isSystemTag, mergeTags, systemTags, userLabels } from '$lib/core/entity-tags';
  import { parseBinding, isEntityKind } from '$lib/core/bindings';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import type { PasswordEntry } from '$lib/pass/types';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    entry?: PasswordEntry | null;
    initialTags?: string[];
    onclose: () => void;
  }

  let { entry = null, initialTags = [], onclose }: Props = $props();

  let title = $state('');
  let username = $state('');
  let url = $state('');
  let password = $state('');
  let note = $state('');
  let noteTouched = $state(false);
  let totpIds = $state<string[]>([]);
  let totpQuery = $state('');
  let totpSuggest = $state(false);
  let addTotp = $state(false);
  let totpName = $state('');
  let totpIssuer = $state('');
  let totpSecret = $state('');
  let tags = $state<string[]>([]);
  let query = $state('');
  let suggest = $state(false);
  let showPassword = $state(false);
  let error = $state('');
  let saving = $state(false);

  const editing = $derived(entry !== null);
  const q = $derived(query.trim().toLowerCase());
  const chosen = $derived(new Set(tags));
  // Labels, profiles, workspaces and notes come through the catalog of entities; a product without their owner offers none.
  const tagHits = $derived(
    directory.list('note_tag').filter((tag) => !userLabels(tags).includes(tag.name) && (!q || tag.name.toLowerCase().includes(q))).slice(0, 8),
  );
  const profileHits = $derived(
    directory.list('profile').filter((profile) => !chosen.has(`profile:${profile.id}`) && (!q || profile.name.toLowerCase().includes(q))).slice(0, 6),
  );
  const workspaceHits = $derived(
    directory.list('workspace').filter((ws) => !chosen.has(`workspace:${ws.id}`) && (!q || ws.name.toLowerCase().includes(q))).slice(0, 6),
  );
  const noteHits = $derived(
    q
      ? directory.list('note').filter((n) => !chosen.has(`note:${n.id}`) && n.name.toLowerCase().includes(q)).slice(0, 6)
      : [],
  );

  onMount(() => {
    void totpStore.ensureLoaded();
    void directory.ensureLoaded();
    if (entry) {
      title = entry.title;
      username = entry.username ?? '';
      url = entry.url ?? '';
      totpIds = [...entry.totp_ids];
      tags = [...entry.tags];
      if (entry.has_note) {
        api.passwords.reveal(entry.id, 'note').then((result) => { note = result.value; }).catch(() => {});
      }
    } else {
      tags = [...initialTags];
    }
  });

  function bindingKind(tag: string): string {
    return parseBinding(tag)?.kind ?? 'tag';
  }

  /** The name of the kind a reference points at ("Profile"), for the chip's tooltip; none for a free label. */
  function bindingCaption(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    const kind = parsed ? directory.kind(parsed.kind) : undefined;
    return kind ? $t(kind.label) : undefined;
  }

  function bindingLabel(tag: string): string {
    const parsed = parseBinding(tag);
    if (!parsed) return tag;
    // The entity's name; a kind without an owner here as 10.3 shows it (its label, else the kind and a short id).
    if (parsed.kind === 'note' || isEntityKind(parsed.kind)) return directory.name(parsed.kind, parsed.value, $t);
    return parsed.value;
  }

  function addLabel(name: string) {
    const value = name.trim();
    if (!value || isSystemTag(value)) return;
    tags = mergeTags([], systemTags(tags), [...userLabels(tags), value]);
    query = '';
  }

  function addBinding(value: string) {
    if (!value || !isSystemTag(value) || tags.includes(value)) return;
    tags = mergeTags([], [...systemTags(tags), value], userLabels(tags));
    query = '';
  }

  async function commitQuery() {
    const value = query.trim();
    if (!value) return;
    try {
      const existing = directory.list('note_tag').find((tag) => tag.name.toLowerCase() === value.toLowerCase());
      if (existing) {
        addLabel(existing.name);
        return;
      }
      const profile = directory.list('profile').filter((item) => item.name.toLowerCase() === value.toLowerCase());
      if (profile.length === 1) {
        addBinding(`profile:${profile[0].id}`);
        return;
      }
      const workspace = directory.list('workspace').filter((item) => item.name.toLowerCase() === value.toLowerCase());
      if (workspace.length === 1) {
        addBinding(`workspace:${workspace[0].id}`);
        return;
      }
      const notes = directory.list('note');
      const titled = notes.filter((n) => n.name.toLowerCase() === value.toLowerCase());
      if (titled.length === 1) {
        addBinding(`note:${titled[0].id}`);
        return;
      }
      const partial = notes.filter((n) => n.name.toLowerCase().includes(value.toLowerCase()));
      if (partial.length === 1) {
        addBinding(`note:${partial[0].id}`);
        return;
      }
      // The owner of the labels makes a new one; without it the label is free text.
      const created = await directory.get('note_tag')?.create?.(value);
      addLabel(created?.name ?? value);
    } catch (e) {
      error = formatError(e);
    }
  }

  function chipColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    if (!parsed) return directory.summary('note_tag', tag)?.color;
    // The owner's color, or the color of a workspace's label where Space is absent (10.3).
    if (parsed.kind === 'workspace') return directory.color('workspace', parsed.value);
    if (parsed.kind === 'profile') return 'var(--accent)';
    return undefined;
  }

  function removeTag(tag: string) {
    if (initialTags.includes(tag) && !editing) return;
    tags = tags.filter((item) => item !== tag);
  }

  function generate() {
    try {
      password = generatePassword(get(pwSettings));
      showPassword = true;
    } catch (e) {
      error = formatError(e);
    }
  }

  async function save() {
    if (saving) return;
    suggest = false;
    error = '';
    if (!title.trim()) {
      error = $t('pw_err_title');
      return;
    }
    if (!editing && !password) {
      error = $t('pw_err_password');
      return;
    }
    saving = true;
    try {
      if (query.trim()) await commitQuery();
      const linked = [...totpIds];
      // Created before the password; removed again if the password save fails.
      let newTotpId: string | null = null;
      if (addTotp && totpSecret.trim()) {
        const created = await api.totp.add({
          name: (totpName || title).trim(),
          issuer: totpIssuer.trim() || null,
          secret: totpSecret.trim(),
          tags,
        });
        newTotpId = created.id;
        linked.push(created.id);
      }
      let savedId = entry?.id;
      try {
        if (entry) {
          await api.passwords.update(entry.id, {
            title: title.trim(),
            username,
            url,
            password: password || null,
            note: noteTouched || note ? note : null,
            clear_note: noteTouched && !note.trim(),
            totp_ids: linked,
            tags,
          });
        } else {
          const created = await api.passwords.create({
            title: title.trim(),
            username: username || null,
            url: url || null,
            password,
            note: note || null,
            totp_ids: linked,
            tags,
          });
          savedId = created.id;
        }
      } catch (e) {
        if (newTotpId) await api.totp.delete(newTotpId).catch(() => {});
        throw e;
      } finally {
        if (newTotpId) await totpStore.refresh();
      }
      await passwordStore.refresh();
      if (savedId) await passwordStore.syncNoteBindings(savedId, tags);
      await appLock.refresh();
      onclose();
    } catch (e) {
      const key = passwordErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<Dialog open title={editing ? $t('pw_edit_title') : $t('cmd_passwords_create')} width="460px" {onclose}>
  <!-- A form: Enter in a field saves, as the Save button does. -->
  <form id="pw-edit-form" class="form" onsubmit={(e) => { e.preventDefault(); void save(); }}>
    {#if error}<div class="error-msg">{error}</div>{/if}
    <div class="form-group">
      <label for="pw-title">{$t('pw_field_title')}</label>
      <input id="pw-title" bind:value={title} autocomplete="off" />
    </div>
    <div class="form-group">
      <label for="pw-user">{$t('pw_field_username')}</label>
      <input id="pw-user" bind:value={username} autocomplete="off" />
    </div>
    <div class="form-group">
      <label for="pw-url">{$t('pw_field_url')}</label>
      <input id="pw-url" bind:value={url} autocomplete="off" />
    </div>
    <div class="form-group">
      <label for="pw-secret">{$t('pw_field_password')}</label>
      <!-- Reveal and generate sit inside the field, so the field keeps its width and its hint fits. -->
      <span class="secret">
        <input
          id="pw-secret"
          class="mono"
          type={showPassword ? 'text' : 'password'}
          bind:value={password}
          autocomplete="new-password"
          placeholder={editing ? $t('pw_unchanged') : ''}
          title={editing ? $t('pw_unchanged') : undefined}
        />
        <button
          type="button"
          class="icon-btn"
          onclick={() => (showPassword = !showPassword)}
          title={$t(showPassword ? 'pw_btn_hide' : 'pw_btn_reveal')}
          aria-label={$t(showPassword ? 'pw_btn_hide' : 'pw_btn_reveal')}
        >
          <Icon name={showPassword ? 'eye-off' : 'eye'} size={14} />
        </button>
        <button type="button" class="icon-btn" onclick={generate} title={$t('pw_btn_generate')} aria-label={$t('pw_btn_generate')}>
          <Icon name="key" size={14} />
        </button>
      </span>
    </div>
    <div class="form-group">
      <label for="pw-note">{$t('pw_field_note')}</label>
      <textarea id="pw-note" bind:value={note} rows="3" oninput={() => (noteTouched = true)}></textarea>
    </div>
    <div class="form-group picker">
      <label for="pw-totp">{$t('pw_field_totp')}</label>
      {#if totpIds.length}
        <div class="chips">
          {#each totpIds as id (id)}
            {@const item = totpStore.list.find((t) => t.id === id)}
            <span class="chip">
              <ChipMark kind="totp" />
              <span class="chip-text">{item ? (item.issuer ? `${item.issuer} · ${item.name}` : item.name) : id}</span>
              <button type="button" class="chip-x" onclick={() => (totpIds = totpIds.filter((x) => x !== id))} aria-label={$t('pass_remove')}>
                <Icon name="x" size={11} />
              </button>
            </span>
          {/each}
        </div>
      {/if}
      {#if !addTotp}
        {@const tq = totpQuery.trim().toLowerCase()}
        {@const totpHits = totpStore.list
          .filter((item) => !totpIds.includes(item.id) && `${item.issuer ?? ''} ${item.name} ${item.tags.join(' ')}`.toLowerCase().includes(tq))
          .slice(0, 8)}
        <span class="row">
          <input
            id="pw-totp"
            bind:value={totpQuery}
            placeholder={$t('totp_search_placeholder')}
            autocomplete="off"
            onfocus={() => (totpSuggest = true)}
            onblur={() => (totpSuggest = false)}
            onkeydown={(e) => {
              // Enter picks the first match instead of saving the form.
              if (e.key !== 'Enter') return;
              e.preventDefault();
              if (totpHits[0]) { totpIds = [...totpIds, totpHits[0].id]; totpQuery = ''; }
            }}
          />
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => { addTotp = true; totpName = title; }}>
            <Icon name="plus" size={12} /> {$t('pw_add_totp')}
          </button>
        </span>
        {#if totpSuggest && totpHits.length}
          <div class="pick" role="presentation">
            {#each totpHits as item (item.id)}
              <button type="button" class="pick-item" onmousedown={(e) => { e.preventDefault(); totpIds = [...totpIds, item.id]; totpQuery = ''; }}>
                <ChipMark kind="totp" />
                {item.issuer ? `${item.issuer} · ${item.name}` : item.name}
              </button>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="draft">
          <input bind:value={totpName} placeholder={$t('totp_field_name')} autocomplete="off" />
          <input bind:value={totpIssuer} placeholder={$t('totp_field_issuer')} autocomplete="off" />
          <input bind:value={totpSecret} placeholder={$t('totp_field_secret')} autocomplete="off" />
          <button type="button" class="btn btn-ghost btn-sm" onclick={() => (addTotp = false)}>{$t('pw_btn_cancel')}</button>
        </div>
      {/if}
    </div>
    <div class="form-group picker">
      <label for="pw-tags">{$t('pw_field_tags')}</label>
      {#if tags.length}
        <div class="chips">
          {#each tags as tag (tag)}
            {@const locked = initialTags.includes(tag) && !editing}
            <!-- A link to an entity this product has no owner for reads as 10.3 shows it: the kind and its name or short id. -->
            <span class="chip tinted" title={bindingCaption(tag)} style:--chip={chipColor(tag)}>
              <ChipMark kind={bindingKind(tag)} caption={bindingCaption(tag)} />
              <span class="chip-text">{bindingLabel(tag)}</span>
              {#if !locked}
                <button type="button" class="chip-x" onclick={() => removeTag(tag)} aria-label={$t('pass_remove')} title={$t('pass_remove')}>
                  <Icon name="x" size={11} />
                </button>
              {/if}
            </span>
          {/each}
        </div>
      {/if}
      <input
        id="pw-tags"
        bind:value={query}
        placeholder={$t('notes_tags_placeholder')}
        autocomplete="off"
        onfocus={() => (suggest = true)}
        onblur={() => (suggest = false)}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); void commitQuery(); } }}
      />
      {#if suggest && (tagHits.length || profileHits.length || workspaceHits.length || noteHits.length)}
        <div class="pick" role="presentation">
          {#each tagHits as tag (tag.id)}
            <button type="button" class="pick-item" onmousedown={(e) => { e.preventDefault(); addLabel(tag.name); }}>
              <span class="dot" style:background={tag.color}></span>
              {tag.name}
            </button>
          {/each}
          {#each profileHits as profile (profile.id)}
            <button type="button" class="pick-item" onmousedown={(e) => { e.preventDefault(); addBinding(`profile:${profile.id}`); }}>
              <ChipMark kind="profile" />
              {profile.name}
            </button>
          {/each}
          {#each workspaceHits as ws (ws.id)}
            <button type="button" class="pick-item" onmousedown={(e) => { e.preventDefault(); addBinding(`workspace:${ws.id}`); }}>
              <span class="dot" style:background={ws.color}></span>
              {ws.name}
            </button>
          {/each}
          {#each noteHits as n (n.id)}
            <button type="button" class="pick-item" onmousedown={(e) => { e.preventDefault(); addBinding(`note:${n.id}`); }}>
              <ChipMark kind="note" />
              {n.name}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </form>
  {#snippet footer()}
    <button class="btn btn-ghost" type="button" onclick={onclose}>{$t('pw_btn_cancel')}</button>
    <!-- mousedown keeps the focus where it is (an open suggestion list stays put); the click saves, from the mouse and the keyboard. -->
    <button class="btn btn-primary" type="submit" form="pw-edit-form" onmousedown={(e) => e.preventDefault()} disabled={saving}>{$t('pw_btn_save')}</button>
  {/snippet}
</Dialog>

<style>
  .form { display: flex; flex-direction: column; gap: var(--sp-4); }
  .picker { position: relative; }
  textarea { resize: vertical; min-height: 72px; }
  .secret { position: relative; display: flex; align-items: center; }
  .secret input { padding-right: calc(2 * 28px + 3 * var(--sp-1) + var(--sp-2)); text-overflow: ellipsis; }
  /* The password is `.mono`, as on its card; the hint stays a sentence in the UI font. */
  .secret input::placeholder { font-family: var(--font-ui); }
  .secret .icon-btn {
    position: absolute;
    width: 28px;
    height: 28px;
    background: transparent;
    border-color: transparent;
  }
  .secret .icon-btn:last-child { right: var(--sp-1); }
  .secret .icon-btn:nth-last-child(2) { right: calc(var(--sp-1) * 2 + 28px); }
  .row { display: flex; gap: var(--sp-2); align-items: center; }
  .row input { flex: 1; min-width: 0; }
  .row .btn { flex-shrink: 0; }
  .draft { display: flex; flex-direction: column; gap: var(--sp-2); }
  .draft .btn { align-self: flex-start; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-1); }
  .chip {
    position: relative;
    cursor: default;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    max-width: 100%;
    border-width: 1px;
    border-style: solid;
    border-radius: var(--radius-sm);
    padding: 2px 4px 2px var(--sp-2);
    font-size: var(--fs-xs);
  }
  /* Only its remove button acts: the chip itself does not light up. */
  .chip:hover:not([style*='--chip']) { border-color: var(--border); color: var(--text-2); }
  .chip-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-xs);
    background: transparent;
    color: inherit;
    opacity: var(--chip-fade);
  }
  .chip-x:hover { opacity: 1; background: var(--surface-hover); color: var(--danger-text); }
  .pick {
    position: absolute;
    z-index: 2;
    left: 0;
    right: 0;
    bottom: 100%;
    display: flex;
    flex-direction: column;
    max-height: 220px;
    overflow: auto;
    padding: var(--sp-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-drawer);
    box-shadow: var(--shadow-lg);
  }
  .pick-item {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: var(--sp-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    text-align: left;
  }
  .pick-item:hover { background: var(--surface-hover); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
</style>
