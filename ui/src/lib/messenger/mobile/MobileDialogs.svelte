<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The report and the servers' dialogs of the phone, once per window (an overlay
  of the module). Not in a screen's frame: a wide window shows two frames, and
  folding it takes the list's frame away with a dialog open in it.
-->
<script lang="ts">
  import { markPhone } from '../shared/phone';
  import ReportHost from '../shared/ReportHost.svelte';
  import ServerAcceptDialog from '../servers/ServerAcceptDialog.svelte';
  import ServerInfoDialog from '../servers/ServerInfoDialog.svelte';
  import PrivateInviteDialog from '../servers/PrivateInviteDialog.svelte';

  // Laid out for a phone, as in the screen they were opened from.
  markPhone();
</script>

<ReportHost />
<ServerAcceptDialog />
<ServerInfoDialog />
<PrivateInviteDialog />
