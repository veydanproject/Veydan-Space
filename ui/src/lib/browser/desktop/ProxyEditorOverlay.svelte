<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The proxy editor another module opened through the catalog (`edit` of the proxy kind), mounted once by the shell. -->
<script lang="ts">
  import ProxyPanel from '$lib/browser/components/ProxyPanel.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { editingProxy, proxyEditor } from '$lib/browser/store/proxy-editor.svelte';
  import type { Proxy } from '$lib/browser/types';

  const proxy = $derived(editingProxy());

  function onsaved(saved: Proxy) {
    proxiesStore.list = proxiesStore.list.map((p) => (p.id === saved.id ? saved : p));
    proxyEditor.close();
  }
</script>

{#if proxy}
  <ProxyPanel {proxy} onclose={() => proxyEditor.close()} {onsaved} />
{/if}
