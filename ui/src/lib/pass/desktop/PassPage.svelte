<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The desktop Pass page (platform-spec 11.7): passwords, TOTP codes and the
  generator side by side — the bodies of the top bar's drawers, rendered as
  panes. A narrow window shows one pane at a time behind a tab bar. In Space
  the drawers stay as quick access from any page and answer the palette and a
  note's card; in Pass there are none, and the panes answer (`passUi.drawers`).
-->
<script lang="ts">
  import { t, type TranslationKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import PasswordDrawer from '$lib/pass/components/PasswordDrawer.svelte';
  import TotpGenerator from '$lib/pass/components/TotpGenerator.svelte';
  import PasswordGenerator from '$lib/pass/components/PasswordGenerator.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { passUi, type PassSection as Section } from '$lib/pass/store/ui.svelte';

  const SECTIONS: { id: Section; title: TranslationKey; icon: string; count?: () => number }[] = [
    { id: 'passwords', title: 'pw_title', icon: 'lock', count: () => passwordStore.list.length },
    { id: 'totp', title: 'totp_title', icon: 'shield', count: () => totpStore.list.length },
    { id: 'generator', title: 'pwgen_title', icon: 'key' },
  ];

  /** Below this width the three panes do not fit and become tabs. */
  const WIDE_PX = 1100;

  let width = $state(0);
  const wide = $derived(width === 0 || width >= WIDE_PX);
  const shown = (id: Section) => wide || passUi.section === id;

  // Without drawers a new password is made in the passwords pane: show it.
  $effect(() => {
    if (!passUi.drawers && (passwordStore.createRequest || passwordStore.openId)) passUi.section = 'passwords';
  });
</script>

<div class="page page--fill pass-page" bind:clientWidth={width}>
  {#if !wide}
    <div class="tab-bar" role="tablist">
      {#each SECTIONS as s (s.id)}
        {@const n = s.count?.() ?? 0}
        <button
          class="tab"
          class:active={passUi.section === s.id}
          role="tab"
          aria-selected={passUi.section === s.id}
          onclick={() => (passUi.section = s.id)}
        >
          <Icon name={s.icon} size={14} />
          {$t(s.title)}
          {#if n > 0}<span class="tab-count">{n}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <div class="panes" class:wide>
    {#if shown('passwords')}
      <div class="slot"><PasswordDrawer as="pane" open /></div>
    {/if}
    {#if shown('totp')}
      <div class="slot"><TotpGenerator as="pane" open /></div>
    {/if}
    {#if shown('generator')}
      <div class="slot"><PasswordGenerator as="pane" open /></div>
    {/if}
  </div>
</div>

<style>
  .pass-page {
    --page-max: 1600px;
  }
  .panes {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--sp-3);
  }
  .panes.wide {
    /* The generator needs the least room, the TOTP rows a little more than it. */
    grid-template-columns: minmax(0, 1.35fr) minmax(300px, 1.1fr) minmax(260px, 0.85fr);
  }
  .slot {
    min-width: 0;
    min-height: 0;
  }
</style>
