<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A request of recovery through the trusted people (`veydan://recover/…`, cloud-recovery.md §8.5): who asks, and the way to «Подтвердить восстановление» of the module cloud, where the code is compared and the talk by voice is confirmed. -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { LinkView } from '../../api';

  interface Props { view: Extract<LinkView, { kind: 'recover' }> }
  let { view }: Props = $props();

  const who = $derived(view.name || `${view.npub.slice(0, 12)}…`);

  function open() {
    void goto(`/cloud/help?npub=${encodeURIComponent(view.npub)}&code=${encodeURIComponent(view.code)}`);
  }
</script>

<div class="box">
  <div class="top">
    <Icon name="key" size={22} />
    <div class="about">
      <span class="name">{$t('cloud_recover_card_title')}</span>
      <span class="sub"><code>{view.code}</code></span>
    </div>
  </div>
  <p class="text">{$t('cloud_recover_card_text', { name: who })}</p>
  <div class="row">
    <button class="btn btn-primary btn-sm" onclick={open}>{$t('cloud_recover_card_open')}</button>
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .sub code { font-family: var(--font-mono); }
  .text { margin: 0; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
</style>
