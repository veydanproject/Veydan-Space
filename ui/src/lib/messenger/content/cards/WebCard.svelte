<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A link of a sign-in to the web cabinet of Veydan Cloud inside a message
  (`veydan://web/…`, cloud-api.md §16.2). It approves nothing from here: a
  sign-in is approved only from the app's own scanner or a paste, so a link
  someone sent cannot sign them in as me. The card says so and offers no
  button.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { LinkView } from '../../api';

  interface Props { view: Extract<LinkView, { kind: 'web' }> }
  let { view: _view }: Props = $props();
</script>

<div class="box">
  <div class="top">
    <span class="ico"><Icon name="alert-triangle" size={18} /></span>
    <div class="about">
      <span class="name">{$t('msg_card_web_title')}</span>
      <span class="sub">{$t('msg_card_web_text')}</span>
    </div>
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; }
  .ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    color: var(--warn-text); background: var(--warn-bg);
  }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
</style>
