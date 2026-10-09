<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api } from '$lib/pass/api';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { passwordErrorKey } from '$lib/core/password-error';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory, foreignName } from '$lib/core/directory';
  import { parseBinding } from '$lib/core/bindings';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import type { PasswordEntry } from '$lib/pass/types';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import VaultUnlock from '$lib/pass/components/VaultUnlock.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';

  interface Props {
    entry: PasswordEntry;
    onedit: () => void;
    ondeleted: () => void;
  }

  let { entry, onedit, ondeleted }: Props = $props();

  let revealed = $state('');
  let note = $state('');
  let error = $state('');
  let toast = $state('');
  let unlockOpen = $state(false);
  let afterUnlock = $state<(() => void) | null>(null);

  const totps = $derived(
    entry.totp_ids
      .map((id) => totpStore.list.find((item) => item.id === id))
      .filter((item): item is NonNullable<typeof item> => !!item),
  );
  const unlocked = $derived(!appLock.locked);

  $effect(() => {
    void directory.ensureLoaded();
    void totpStore.ensureLoaded();
  });

  $effect(() => {
    if (!unlocked) {
      revealed = '';
      note = '';
      return;
    }
    if (!entry.has_note) {
      note = '';
      return;
    }
    const id = entry.id;
    let cancelled = false;
    api.passwords.reveal(id, 'note').then((result) => {
      if (!cancelled) note = result.value;
    }).catch(() => {});
    return () => { cancelled = true; };
  });

  function tagColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    if (!parsed) return directory.summary('note_tag', tag)?.color;
    // The owner's color, or the color of a workspace's label where Space is absent (10.3).
    if (parsed.kind === 'workspace') return directory.color('workspace', parsed.value);
    if (parsed.kind === 'profile') return 'var(--accent)';
    return undefined;
  }

  function tagLabel(tag: string): string {
    const parsed = parseBinding(tag);
    if (!parsed) return tag;
    // A kind without an owner in this product: the name its label keeps, else the kind's name and a short id (10.3).
    const foreign = directory.foreign(parsed.kind, parsed.value);
    if (foreign) return foreignName(foreign, $t);
    if (parsed.kind === 'profile' || parsed.kind === 'workspace' || parsed.kind === 'note') {
      return directory.summary(parsed.kind, parsed.value)?.name ?? parsed.value;
    }
    return parsed.value;
  }

  /** The name of the kind a reference points at ("Profile"), for the chip's tooltip; none for a free label. */
  function tagKind(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    const kind = parsed ? directory.kind(parsed.kind) : undefined;
    return kind ? $t(kind.label) : undefined;
  }

  function fail(e: unknown) {
    const key = passwordErrorKey(e);
    error = key ? $t(key) : formatError(e);
  }

  async function reveal() {
    error = '';
    try {
      if (revealed) {
        revealed = '';
        return;
      }
      revealed = (await api.passwords.reveal(entry.id, 'password')).value;
    } catch (e) {
      fail(e);
    }
  }

  function askUnlock(then: () => void) {
    afterUnlock = then;
    unlockOpen = true;
  }

  function onUnlocked() {
    const next = afterUnlock;
    afterUnlock = null;
    next?.();
  }

  function promote(totpId: string) {
    const run = () => void passwordStore.makePrimaryTotp(entry.id, totpId).catch(fail);
    if (appLock.locked) askUnlock(run);
    else run();
  }

  async function copyPassword() {
    if (appLock.locked) {
      askUnlock(() => void copyPassword());
      return;
    }
    error = '';
    try {
      await api.passwords.copy(entry.id);
      toast = $t('pw_copied');
    } catch (e) {
      const code = e && typeof e === 'object' && 'code' in e ? String((e as { code: string }).code) : '';
      if (code === 'vault_locked' || code === 'vault_mismatch' || code === 'decrypt_failed') {
        askUnlock(() => void copyPassword());
        return;
      }
      try {
        const value = (await api.passwords.reveal(entry.id, 'password')).value;
        await navigator.clipboard.writeText(value);
        toast = $t('pw_copied');
        setTimeout(() => {
          navigator.clipboard.readText().then((current) => {
            if (current === value) void navigator.clipboard.writeText('');
          }).catch(() => {});
        }, 30_000);
      } catch (err) {
        fail(err);
      }
    }
  }

  async function copyUsername() {
    if (!entry.username) return;
    await navigator.clipboard.writeText(entry.username);
    toast = $t('pw_copied');
  }

  async function remove() {
    if (!(await ask({ title: $t('pw_delete_confirm', { name: entry.title }) }))) return;
    try {
      await api.passwords.delete(entry.id);
      await passwordStore.refresh();
      ondeleted();
    } catch (e) {
      fail(e);
    }
  }
</script>

<article class="card">
  <h2>{entry.title}</h2>
  {#if entry.username}
    <div class="line">
      <span>{$t('pw_field_username')}</span>
      <strong>{entry.username}</strong>
      <button type="button" class="btn btn-ghost btn-sm" onclick={copyUsername}>{$t('pw_btn_copy')}</button>
    </div>
  {/if}
  {#if entry.url}
    <div class="line">
      <span>{$t('pw_field_url')}</span>
      <a href={entry.url} target="_blank" rel="noreferrer">{entry.url}</a>
    </div>
  {/if}
  <div class="line">
    <span>{$t('pw_field_password')}</span>
    <strong class="mono">{unlocked && revealed ? revealed : '••••••••'}</strong>
    {#if unlocked}
      <button type="button" class="icon-btn" onclick={reveal} aria-label={$t(revealed ? 'pw_btn_hide' : 'pw_btn_reveal')}>
        <Icon name={revealed ? 'eye-off' : 'eye'} size={14} />
      </button>
      <button type="button" class="icon-btn" class:ok={toast === $t('pw_copied')} onclick={copyPassword} aria-label={$t('pw_btn_copy')}>
        <Icon name={toast === $t('pw_copied') ? 'check' : 'copy'} size={14} />
      </button>
    {:else}
      <button type="button" class="icon-btn" onclick={() => askUnlock(() => void copyPassword())} aria-label={$t('lock_title')}>
        <Icon name="lock" size={14} />
      </button>
    {/if}
  </div>
  {#if note}
    <div class="line">
      <span>{$t('pw_field_note')}</span>
      <p>{note || '—'}</p>
    </div>
  {/if}
  {#if entry.tags.length}
    <div class="chips">
      {#each entry.tags as tag (tag)}
        {@const color = tagColor(tag)}
        <span class="chip tinted" title={tagKind(tag)} style:--chip={color}>
          <ChipMark kind={parseBinding(tag)?.kind ?? 'tag'} caption={tagKind(tag)} />
          <span class="chip-text">{tagLabel(tag)}</span>
        </span>
      {/each}
    </div>
  {/if}
  {#each totps as totp, i (totp.id)}
    <div class="line">
      <span>{$t('pw_field_totp')}</span>
      <strong class="totp-name">{totp.issuer ? `${totp.issuer} · ${totp.name}` : totp.name}</strong>
      {#if totps.length > 1}
        {#if i === 0}
          <span class="primary" title={$t('pw_totp_primary')}><Icon name="pin" size={12} /></span>
        {:else}
          <button type="button" class="icon-btn" title={$t('pw_totp_make_primary')} onclick={() => promote(totp.id)}>
            <Icon name="arrow-up" size={12} />
          </button>
        {/if}
      {/if}
      <TotpLiveCode entryId={totp.id} period={totp.period} copiedLabel={$t('totp_copy')} compact />
    </div>
  {/each}
  {#if error}<p class="err">{error}</p>{/if}
  {#if toast}<p class="ok">{toast}</p>{/if}
  <VaultUnlock bind:open={unlockOpen} onunlocked={onUnlocked} />
  {#if unlocked}
    <div class="actions">
      <button type="button" class="btn btn-ghost btn-sm" onclick={onedit}><Icon name="pencil" size={12} /> {$t('pw_btn_edit')}</button>
      <button type="button" class="btn btn-ghost btn-sm" onclick={remove}><Icon name="trash-2" size={12} /> {$t('pw_btn_delete')}</button>
    </div>
  {/if}
</article>

<style>
  .card { display: flex; flex-direction: column; gap: var(--sp-3); }
  /* A long value without spaces wraps inside the card instead of running past its border */
  h2 { margin: 0; font-size: 1.05rem; min-width: 0; overflow-wrap: anywhere; }
  .line { display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-2); font-size: 0.85rem; }
  /* The row label only (the pin mark is a span too) */
  .line > span:first-child { color: var(--text-2); min-width: 72px; }
  .line > strong, .line > a, .line > p { min-width: 0; max-width: 100%; overflow-wrap: anywhere; }
  .line > p { white-space: pre-wrap; }
  .totp-name { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .primary { display: inline-flex; color: var(--accent); }
  a { color: var(--text); }
  .err { color: var(--danger-text); margin: 0; }
  .ok { color: var(--success-text); margin: 0; }
  .icon-btn.ok { color: var(--success-text); }
  .actions { display: flex; gap: var(--sp-2); }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  /* Shown, not clicked: a link to an entity of another module goes nowhere here. */
  .chip {
    position: relative;
    cursor: default;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border-width: 1px;
    border-style: solid;
    border-radius: 99px;
    max-width: 100%;
    padding: 2px 8px;
    font-size: 0.75rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* The chip is inline-flex, so the ellipsis goes on the label itself */
  .chip-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  /* Not a toggle: the hover of the global .chip changes nothing here. */
  .chip:hover:not([style*='--chip']) { border-color: var(--border); color: var(--text-2); }
</style>
