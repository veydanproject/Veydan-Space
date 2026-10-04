<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The pass module's global drawers, mounted once by the desktop shell of Space:
  the generator, TOTP, passwords. Pass has none (`quick`): its window is the
  Pass page.
-->
<script lang="ts">
  import PasswordGenerator from '$lib/pass/components/PasswordGenerator.svelte';
  import TotpGenerator from '$lib/pass/components/TotpGenerator.svelte';
  import PasswordDrawer from '$lib/pass/components/PasswordDrawer.svelte';
  import { passUi } from '$lib/pass/store/ui.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';

  // While the drawers are here (Space), requests open them instead of the Pass page's panes.
  $effect(() => {
    passUi.drawers = true;
    return () => {
      passUi.drawers = false;
    };
  });

  // A request from the palette or a note's context card opens the drawer it is for.
  $effect(() => {
    if (passwordStore.createRequest || passwordStore.openId) passUi.passwordsOpen = true;
  });
  $effect(() => {
    if (totpStore.pendingSearch !== null) passUi.totpOpen = true;
  });
</script>

<PasswordGenerator bind:open={passUi.generatorOpen} />
<TotpGenerator bind:open={passUi.totpOpen} context="global" />
<PasswordDrawer bind:open={passUi.passwordsOpen} context="global" />
