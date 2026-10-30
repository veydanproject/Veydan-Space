<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Which private server of mine (`value`, its id; `null`: the first choice
  `none`, the home server or no server at all, when it is offered). A few
  are chips; many — a company may have dozens, a person a hundred of
  groups' servers — are one field that opens a list with a search by name
  and host. «Пригласить в частный чат», «Мой QR», a card of mine sent with
  a server and «Новая группа» choose with it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { hostOf } from './serverAccept.svelte';
  import { nameOfServer, serverStore } from './serverStore.svelte';

  interface Props {
    value: string | null;
    /** The label of the choice without a private server; none: a server must be chosen. */
    none?: string | null;
    disabled?: boolean;
    label?: string;
  }
  let { value = $bindable(), none = null, disabled = false, label = '' }: Props = $props();

  /** More than this many choices: a field with a list and a search. */
  const CHIPS_MAX = 4;
  const servers = $derived(serverStore.list ?? []);
  const many = $derived(servers.length + (none !== null ? 1 : 0) > CHIPS_MAX);
  const chosen = $derived(servers.find((s) => s.id === value) ?? null);

  let open = $state(false);
  let query = $state('');
  let search = $state<HTMLInputElement | null>(null);
  const found = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return servers;
    return servers.filter((s) => nameOfServer(s).toLowerCase().includes(q) || hostOf(s.relay).toLowerCase().includes(q));
  });

  function pick(id: string | null) {
    value = id;
    open = false;
    query = '';
  }

  $effect(() => { if (open) search?.focus(); });
</script>

{#if !many}
  <div class="chips" role="radiogroup" aria-label={label || $t('msg_srv_row_server')}>
    {#if none !== null}
      <button type="button" class="srv-chip" class:on={value === null} role="radio" aria-checked={value === null} {disabled} onclick={() => pick(null)}>{none}</button>
    {/if}
    {#each servers as s (s.id)}
      <button type="button" class="srv-chip" class:on={value === s.id} role="radio" aria-checked={value === s.id} {disabled} onclick={() => pick(s.id)}>
        <Icon name="server-2" size={12} /><span class="srv-chip-text">{nameOfServer(s)}</span>
      </button>
    {/each}
  </div>
{:else}
  <div class="many">
    <button type="button" class="field" aria-expanded={open} aria-label={label || $t('msg_srv_row_server')} {disabled} onclick={() => (open = !open)}>
      <Icon name={chosen ? 'server-2' : 'world'} size={14} />
      <span class="field-text">
        <span class="name">{chosen ? nameOfServer(chosen) : (none ?? '')}</span>
        {#if chosen}<span class="host">{hostOf(chosen.relay)}</span>{/if}
      </span>
      <span class="count">{servers.length}</span>
      <Icon name={open ? 'chevron-up' : 'chevron-down'} size={14} />
    </button>
    {#if open}
      <div class="list">
        <input bind:this={search} class="search" type="text" bind:value={query} placeholder={$t('msg_srv_pick_search')} spellcheck="false" autocomplete="off"
          onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); open = false; } if (e.key === 'Enter' && found.length === 1) pick(found[0].id); }} />
        <ul role="listbox" aria-label={label || $t('msg_srv_row_server')}>
          {#if none !== null && !query.trim()}
            <li><button type="button" class="opt" class:on={value === null} role="option" aria-selected={value === null} onclick={() => pick(null)}>
              <Icon name="world" size={14} /><span class="name">{none}</span>
            </button></li>
          {/if}
          {#each found as s (s.id)}
            <li><button type="button" class="opt" class:on={value === s.id} role="option" aria-selected={value === s.id} onclick={() => pick(s.id)}>
              <Icon name="server-2" size={14} /><span class="name">{nameOfServer(s)}</span><span class="host">{hostOf(s.relay)}</span>
            </button></li>
          {:else}
            <li class="empty">{$t('msg_srv_pick_none')}</li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
{/if}

<style>
  .chips { display: flex; gap: var(--sp-1); flex-wrap: wrap; }
  /* A chip of the design book: 32 high, 13 semibold, the chosen one in the accent. */
  .srv-chip {
    display: inline-flex; align-items: center; gap: var(--sp-1); max-width: 100%; min-width: 0; min-height: 32px;
    font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); cursor: pointer;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-pill); padding: 0 var(--sp-3);
    transition: border-color var(--dur-fast), color var(--dur-fast);
  }
  .srv-chip-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .srv-chip:hover:not(:disabled) { border-color: var(--border-2); color: var(--text); }
  .srv-chip.on { background: var(--accent-bg); color: var(--accent-text); border-color: var(--accent-border); }
  .srv-chip:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .srv-chip:disabled { opacity: 0.5; cursor: default; }

  .many { display: flex; flex-direction: column; gap: 6px; }
  .field {
    display: flex; align-items: center; gap: var(--sp-2); width: 100%; min-width: 0; text-align: start; cursor: pointer;
    font: inherit; color: var(--text); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 7px 10px;
  }
  .field :global(svg) { flex-shrink: 0; color: var(--text-3); }
  .field-text { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .host { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: var(--fs-2xs); color: var(--text-3); padding: 0 var(--sp-2); border-radius: var(--radius-pill); background: var(--surface-2); }
  .list { display: flex; flex-direction: column; gap: 4px; padding: 6px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--surface-2); }
  .search {
    font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 6px 9px;
  }
  .search:focus { outline: none; border-color: var(--accent-border); }
  ul { list-style: none; margin: 0; padding: 0; max-height: 220px; overflow-y: auto; }
  .opt {
    display: flex; align-items: center; gap: var(--sp-2); width: 100%; min-width: 0; text-align: start; cursor: pointer;
    font: inherit; color: var(--text-2); background: none; border: 0; border-radius: var(--radius-sm); padding: 6px 8px;
  }
  .opt :global(svg) { flex-shrink: 0; color: var(--text-3); }
  .opt .name { flex: 1; min-width: 0; color: var(--text); }
  .opt .host { flex-shrink: 1; min-width: 0; max-width: 50%; }
  .opt:hover { background: var(--surface-3); }
  .opt.on { background: var(--accent-tint); }
  .opt.on .name { color: var(--accent-text-2); }
  .empty { padding: 8px; font-size: var(--fs-xs); color: var(--text-3); }
  @media (pointer: coarse) { .srv-chip { min-height: 40px; padding: 0 var(--sp-4); } .opt { padding: 10px 8px; } .search { font-size: var(--fs-md); } }
</style>
