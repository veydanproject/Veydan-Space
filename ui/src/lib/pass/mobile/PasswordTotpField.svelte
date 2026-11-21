<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Linked TOTP codes of a password: pick existing ones or draft a new one. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { AVATAR_COLORS } from '$lib/core/mobile/nav-colors';
  import type { TotpEntry } from '$lib/pass/types';

  interface Props {
    entries: TotpEntry[];
    totpIds?: string[];
    creating?: boolean;
    totpName?: string;
    totpIssuer?: string;
    totpSecret?: string;
    suggestName?: string;
  }

  let {
    entries,
    totpIds = $bindable([]),
    creating = $bindable(false),
    totpName = $bindable(''),
    totpIssuer = $bindable(''),
    totpSecret = $bindable(''),
    suggestName = '',
  }: Props = $props();
  const uid = $props.id();

  let open = $state(false);
  let query = $state('');

  const linked = $derived(
    totpIds.map((id) => entries.find((item) => item.id === id)).filter((item): item is TotpEntry => !!item),
  );
  const q = $derived(query.trim().toLowerCase());
  const hits = $derived(
    entries.filter((item) => !totpIds.includes(item.id) && `${item.issuer ?? ''} ${item.name} ${item.tags.join(' ')}`.toLowerCase().includes(q)),
  );

  function labelOf(item: TotpEntry): string {
    return item.issuer ? `${item.issuer} · ${item.name}` : item.name;
  }

  function colorOf(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AVATAR_COLORS[h % AVATAR_COLORS.length];
  }

  function add(id: string) {
    if (!totpIds.includes(id)) totpIds = [...totpIds, id];
    open = false;
  }

  function remove(id: string) {
    totpIds = totpIds.filter((item) => item !== id);
  }

  function startCreate() {
    creating = true;
    if (!totpName) totpName = suggestName;
    open = false;
  }

  function cancelCreate() {
    creating = false;
    totpSecret = '';
  }
</script>

<!-- An .m-field like the fields around it: its label names the group, as no single control is labelled. -->
<div class="m-field" role="group" aria-labelledby="{uid}-label">
  <span class="form-label" id="{uid}-label">{$t('pw_field_totp')}</span>
  {#each linked as item (item.id)}
    {@const label = item.issuer || item.name}
    <div class="picked">
      <span class="m-avatar" style:--c={colorOf(label)}>{label.charAt(0).toUpperCase()}</span>
      <span class="meta">
        <span class="name">{label}</span>
        {#if item.issuer}<span class="sub">{item.name}</span>{/if}
      </span>
      <button type="button" class="icon-btn" onclick={() => remove(item.id)} aria-label={$t('pass_remove')}>
        <Icon name="x" size={22} />
      </button>
    </div>
  {/each}
  {#if creating}
    <div class="draft">
      <input bind:value={totpName} placeholder={$t('totp_field_name')} aria-label={$t('totp_field_name')} autocomplete="off" enterkeyhint="next" />
      <input bind:value={totpIssuer} placeholder={$t('totp_field_issuer')} aria-label={$t('totp_field_issuer')} autocomplete="off" enterkeyhint="next" />
      <!-- The key as in the TOTP page's field: capitals, no corrections. -->
      <input class="mono" bind:value={totpSecret} placeholder={$t('totp_field_secret_placeholder')} aria-label={$t('totp_field_secret')}
        autocomplete="off" autocapitalize="characters" spellcheck="false" />
      <button type="button" class="m-link cancel" onclick={cancelCreate}>{$t('common_cancel')}</button>
    </div>
  {:else}
    <button type="button" class="m-link add" onclick={() => (open = true)}>
      <Icon name="plus" size={18} /> {$t('pw_add_totp')}
    </button>
  {/if}
</div>

<BottomSheet {open} title={$t('pw_field_totp')} share={0.45} onclose={() => (open = false)}>
  <div class="m-list create">
    <button type="button" class="list-row dense" onclick={startCreate}>
      <Icon name="plus" size={18} />
      <span class="list-row-label">{$t('totp_pick_new')}</span>
    </button>
  </div>
  {#if entries.length === 0}
    <p class="empty-state sm">{$t('totp_empty')}</p>
  {:else}
    <input class="find" bind:value={query} placeholder={$t('totp_search_placeholder')} autocomplete="off" />
    {#if hits.length}
      <div class="m-list hits">
        {#each hits as item (item.id)}
          {@const label = item.issuer || item.name}
          <button type="button" class="list-row dense" onclick={() => add(item.id)}>
            <span class="m-avatar small" style:--c={colorOf(label)}>{label.charAt(0).toUpperCase()}</span>
            <span class="list-row-label">{labelOf(item)}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty-state sm">{q ? $t('common_nothing_found') : $t('totp_empty')}</p>
    {/if}
  {/if}
</BottomSheet>

<style>
  .add { align-self: flex-start; padding: 0; }
  .picked {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 64px;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }
  .meta { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .name { font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .draft { display: flex; flex-direction: column; gap: var(--sp-2); }
  .cancel { align-self: flex-start; padding: 0; color: var(--text-2); }
  .create { margin-bottom: var(--sp-3); }
  .find { width: 100%; margin-bottom: var(--sp-3); }
  /* The suggestions of a phone: rows of 44; the sheet keeps to 45% of the screen (share), they scroll inside it. */
  .hits.hits { flex: 0 1 auto; min-height: 0; overflow-y: auto; overscroll-behavior: contain; }
  .hits .list-row { min-height: 44px; }
  .m-avatar.small { width: 28px; height: 28px; font-size: var(--fs-sm); }
</style>
