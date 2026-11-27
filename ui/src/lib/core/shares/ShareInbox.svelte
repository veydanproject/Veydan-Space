<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Where the router of shares meets the modules that take them
  (`ModuleDef.shares`). The module of the lowest `order` shows the share
  (the messenger: its chooser of a chat; the notes alone: a new note); the
  others' `alt` are offered beside it («Новая заметка» in the chooser).
  Mounted by the mobile shell while the app is unlocked: a share that came
  while it was locked waits with the shell until then.
-->
<script lang="ts">
  import { registry } from '../registry';
  import { shareRouter } from './router';
  import { sharesApi } from './api';

  const handlers = $derived(
    registry.active.flatMap((m) => (m.shares ? [m.shares] : [])).toSorted((a, b) => a.order - b.order),
  );
  const primary = $derived(handlers[0] ?? null);
  const others = $derived(handlers.slice(1).flatMap((h) => (h.alt ? [h.alt] : [])));

  $effect(() => {
    const shows = primary;
    if (!shows) return;
    const alts = others;
    return shareRouter.provide(async () => {
      if (!shows.ready()) return false;
      const share = await sharesApi.take();
      if (share) await shows.show(share, alts);
      return true;
    });
  });

  // The module became able to show (its runtime started): a share may wait for it.
  $effect(() => {
    if (primary?.ready()) void shareRouter.pull();
  });
</script>
