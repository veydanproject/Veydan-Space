<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The frame of a screen of the cloud that opens from another one (a device, the recovery, the trusted people): a header with the way back and one card of content, the same on a computer and a phone. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isMobile } from '$lib/core/platform';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  let { title, back, children }: { title: string; back: string; children: Snippet } = $props();
</script>

{#if isMobile}
  <div class="m-page">
    <div class="m-header">
      <a class="m-ibtn" href={back} aria-label={$t('cloud_link_back')}>
        <Icon name="chevron-left" size={24} />
      </a>
      <h1 class="m-title" class:long={title.length > 24}>{title}</h1>
    </div>
    <div class="m-body">
      <div class="m-card sheet">{@render children()}</div>
    </div>
  </div>
{:else}
  <div class="page screen">
    <div class="page-header">
      <a class="icon-btn" href={back} aria-label={$t('cloud_link_back')}><Icon name="chevron-left" size={18} /></a>
      <h1>{title}</h1>
    </div>
    <div class="card body">{@render children()}</div>
  </div>
{/if}

<style>
  .screen { --page-max: 600px; }
  .body, .sheet { display: flex; flex-direction: column; gap: var(--sp-4); }
  .body { padding: var(--sp-6); }
  .sheet { padding: var(--sp-4); margin-top: var(--sp-2); }
  /* A long title keeps all its words: smaller and on two lines, never cut. */
  .m-title.long { font-size: 17px; line-height: 1.2; white-space: normal; overflow-wrap: anywhere; }
</style>
