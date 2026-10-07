<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A person's links elsewhere, as the runtime checked them: a mark of the
  platform and the handle. A press opens the profile in the system
  browser; a link without an address (a Discord name) is copied instead.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { SocialView } from '../api';
  import { linkActions } from '../content/actions';
  import type { ExternalUrl } from '../content/types';
  import { brandIcon } from './brands';

  interface Props {
    socials: SocialView[];
    /** Smaller marks and padding (a card in a chat). */
    compact?: boolean;
  }
  let { socials, compact = false }: Props = $props();

  let copied = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function open(s: SocialView) {
    if (/^https:\/\/\S+$/.test(s.url)) {
      linkActions.openExternal(s.url as ExternalUrl).catch(() => {});
      return;
    }
    linkActions.copy(s.handle).then(() => {
      copied = s.platform + s.handle;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => (copied = null), 1500);
    }).catch(() => {});
  }
</script>

{#if socials.length}
  <div class="socials" class:compact>
    {#each socials as s (s.platform + s.handle)}
      {@const done = copied === s.platform + s.handle}
      <button type="button" class="social" title={s.url ? `${s.name}: ${s.url}` : `${s.name}: ${$t('msg_card_copy')}`} onclick={() => open(s)}>
        <Icon name={done ? 'check' : brandIcon(s.platform)} size={compact ? 13 : 15} />
        <span class="handle">{done ? $t('msg_card_copied') : s.handle}</span>
      </button>
    {/each}
  </div>
{/if}


<style>
  .socials { display: flex; flex-wrap: wrap; gap: 6px; min-width: 0; }
  .social {
    display: inline-flex; align-items: center; gap: 6px; max-width: 100%; min-width: 0;
    padding: 4px 10px; border-radius: 999px; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border);
    transition: background 0.15s var(--ease), border-color 0.15s var(--ease);
  }
  .social:hover { background: var(--surface-hover); border-color: var(--border-2, var(--border)); }
  .social:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .handle { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compact .social { padding: 3px 8px; gap: 5px; }
</style>
