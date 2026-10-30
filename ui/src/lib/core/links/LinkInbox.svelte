<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Where the router of links meets the module that shows them
  (`ModuleDef.links`). Mounted by both shells while the app is unlocked: a
  link that came while it was locked waits with the shell until then.
-->
<script lang="ts">
  import { registry } from '../registry';
  import { linkRouter } from './router';

  const reader = $derived(registry.active.find((m) => m.links)?.links ?? null);

  $effect(() => {
    const r = reader;
    if (!r) return;
    return linkRouter.provide(r.read);
  });

  // The module became able to read (its runtime started): a link may wait for it.
  $effect(() => {
    if (reader?.ready()) void linkRouter.pull();
  });
</script>
