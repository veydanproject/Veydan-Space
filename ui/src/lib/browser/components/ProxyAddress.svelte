<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { Proxy, ProxyCheckResult } from '$lib/browser/types';
  import { isTor, torLabel } from '$lib/browser/proxy-label';

  interface Props {
    proxy: Proxy;
    /** The last check made on this page: running, its answer or its (explained) error. */
    result?: (ProxyCheckResult & { checking?: boolean; err?: string }) | undefined;
  }

  let { proxy, result }: Props = $props();
</script>

{#if isTor(proxy)}
  <code>{torLabel(proxy.country, $t)}</code>
  {#if result?.checking}
    <div class="tor-line">{$t('proxy_tor_checking')}</div>
  {:else if result?.err}
    <div class="tor-line tor-line--error">{result.err}</div>
  {:else if result?.ok && result.ip}
    <div class="tor-line" title={$t('proxy_tor_ip_hint')}>
      {$t('proxy_tor_exit_ip', { ip: result.ip })}
      <span class="tor-hint">{$t('proxy_tor_ip_hint')}</span>
    </div>
  {/if}
{:else}
  <code>{proxy.host}:{proxy.port}</code>
{/if}

<style>
  code {
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body);
    background: var(--surface-2); padding: 5px 10px; border-radius: 7px;
  }
  .tor-line { margin-top: 2px; font-size: var(--fs-2xs); color: var(--text-dim); white-space: normal; }
  .tor-line--error { color: var(--danger-text); }
  .tor-hint { display: block; color: var(--text-faint); }
</style>
