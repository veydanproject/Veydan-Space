<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My links elsewhere: a row per link (the platform's mark and name, the
  handle as typed), "+" to add one: pick a platform, then type a handle,
  an @handle or paste the profile's address. The runtime checks and
  normalizes them when the profile is published.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { messengerApi, SOCIALS_MAX, type SocialLink, type SocialPlatform } from '../api';
  import { brandIcon } from './brands';

  interface Props {
    socials: SocialLink[];
    disabled?: boolean;
    /** A refusal of the last save about the links. */
    error?: string;
  }
  let { socials = $bindable(), disabled = false, error = '' }: Props = $props();

  let platforms = $state<SocialPlatform[]>([]);
  let dialog = $state(false);
  let step = $state<'pick' | 'handle'>('pick');
  let query = $state('');
  let chosen = $state<SocialPlatform | null>(null);
  let handle = $state('');
  /** The row being changed; `null` while adding. */
  let editing = $state<number | null>(null);
  let handleInput = $state<HTMLInputElement | null>(null);

  // The list of platforms is the runtime's; asked once per editor.
  $effect(() => {
    messengerApi.profiles.platforms().then((p) => (platforms = p)).catch(() => {});
  });

  const byId = $derived(new Map(platforms.map((p) => [p.id, p])));
  const nameOf = (id: string) => (id === 'other' ? $t('msg_profile_socials_other') : byId.get(id)?.name ?? id);
  const full = $derived(socials.length >= SOCIALS_MAX);
  const q = $derived(query.trim().toLowerCase());
  const shown = $derived(platforms.filter((p) => !q || p.name.toLowerCase().includes(q) || p.id.includes(q)));

  function add() {
    if (disabled || full) return;
    editing = null; chosen = null; handle = ''; query = ''; step = 'pick';
    dialog = true;
  }

  function change(i: number) {
    if (disabled) return;
    const s = socials[i];
    editing = i;
    chosen = byId.get(s.p) ?? { id: s.p, name: nameOf(s.p), hint: '' };
    handle = s.h; step = 'handle';
    dialog = true;
    void focusHandle();
  }

  async function focusHandle() {
    await tick();
    handleInput?.focus();
  }

  function choose(p: SocialPlatform) {
    chosen = p; step = 'handle';
    void focusHandle();
  }

  function back() {
    if (editing !== null) { dialog = false; return; }
    step = 'pick'; chosen = null;
  }

  function commit() {
    const h = handle.trim();
    if (!chosen || !h) return;
    const link: SocialLink = { p: chosen.id, h };
    if (editing !== null) {
      socials = socials.map((s, i) => (i === editing ? link : s));
    } else if (!socials.some((s) => s.p === link.p && s.h.trim().toLowerCase() === h.toLowerCase())) {
      socials = [...socials, link];
    }
    dialog = false;
  }

  function remove(i: number) {
    socials = socials.filter((_, j) => j !== i);
  }
</script>

<div class="social-editor">
  {#if socials.length}
    <ul class="rows">
      {#each socials as s, i (i)}
        <li class="row">
          <button type="button" class="main" {disabled} onclick={() => change(i)} title={nameOf(s.p)}>
            <span class="mark"><Icon name={brandIcon(s.p)} size={16} /></span>
            <span class="text">
              <span class="pname">{nameOf(s.p)}</span>
              <span class="handle">{s.h}</span>
            </span>
          </button>
          <button type="button" class="icon-btn remove" {disabled} onclick={() => remove(i)} title={$t('msg_profile_socials_remove')} aria-label={$t('msg_profile_socials_remove')}>
            <Icon name="x" size={14} />
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="hint">{$t('msg_profile_socials_empty')}</p>
  {/if}
  {#if error}<div class="field-error" role="alert">{error}</div>{/if}
  <div class="foot">
    <button type="button" class="btn btn-ghost btn-sm" disabled={disabled || full} onclick={add}>
      <Icon name="plus" size={13} />{$t('msg_profile_socials_add')}
    </button>
    {#if full}<span class="hint">{$t('msg_profile_socials_limit', { n: String(SOCIALS_MAX) })}</span>{/if}
  </div>
</div>

<Dialog bind:open={dialog} title={step === 'pick' ? $t('msg_profile_socials_pick') : chosen ? nameOf(chosen.id) : ''} width="min(460px, calc(100vw - 24px))">
  {#if step === 'pick'}
    <div class="pick">
      <input type="search" bind:value={query} placeholder={$t('msg_profile_socials_search')} spellcheck="false" aria-label={$t('msg_profile_socials_search')} />
      <div class="grid">
        {#each shown as p (p.id)}
          <button type="button" class="platform" onclick={() => choose(p)}>
            <span class="pmark"><Icon name={brandIcon(p.id)} size={20} /></span>
            <span class="plabel">{p.id === 'other' ? $t('msg_profile_socials_other') : p.name}</span>
          </button>
        {/each}
      </div>
    </div>
  {:else if chosen}
    <form class="handle-form" onsubmit={(e) => { e.preventDefault(); commit(); }}>
      <label class="lbl" for="social-handle">{$t('msg_profile_socials_handle')}</label>
      <input id="social-handle" type="text" bind:this={handleInput} bind:value={handle} placeholder={chosen.hint} spellcheck="false" autocapitalize="off" autocomplete="off" />
      {#if chosen.hint}<p class="hint">{$t('msg_profile_socials_handle_hint', { hint: chosen.hint })}</p>{/if}
    </form>
  {/if}
  {#snippet footer()}
    {#if step === 'handle'}
      <button type="button" class="btn btn-ghost" onclick={back}>{$t('msg_back')}</button>
      <button type="button" class="btn btn-primary" disabled={!handle.trim()} onclick={commit}>
        <Icon name="check" size={14} />{editing !== null ? $t('common_confirm') : $t('msg_profile_socials_add')}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .social-editor { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .main {
    flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 6px 10px; cursor: pointer; text-align: left;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); color: var(--text); font: inherit;
  }
  .main:hover:not(:disabled) { border-color: var(--border-2); background: var(--surface-hover); }
  .main:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .mark, .pmark {
    width: 30px; height: 30px; border-radius: var(--radius-sm); flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center; background: var(--surface-2); color: var(--text);
  }
  .text { display: flex; flex-direction: column; min-width: 0; }
  .pname { font-size: var(--fs-2xs); color: var(--text-3); font-weight: var(--fw-semibold); }
  .handle { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .remove { flex-shrink: 0; }
  .foot { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .btn :global(svg) { margin-right: 2px; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); }
  .field-error { font-size: var(--fs-xs); color: var(--danger-text); }
  .pick { display: flex; flex-direction: column; gap: var(--sp-3); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: var(--sp-2); }
  .platform {
    display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 10px 4px; cursor: pointer;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
    color: var(--text); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); text-align: center;
  }
  .platform:hover { border-color: var(--accent-tint-border); background: var(--surface-hover); }
  .platform:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .platform .pmark { background: var(--surface-3); }
  .plabel { overflow: hidden; text-overflow: ellipsis; max-width: 100%; white-space: nowrap; }
  .handle-form { display: flex; flex-direction: column; gap: var(--sp-2); }
  .lbl { font-size: var(--fs-xs); color: var(--text-3); }
  @media (pointer: coarse) {
    .remove { width: 40px; height: 40px; }
    .main { min-height: 44px; }
    .grid { grid-template-columns: repeat(auto-fill, minmax(84px, 1fr)); }
  }
</style>
