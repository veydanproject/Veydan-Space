<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The in-window notice cards on a computer, mounted once by the shell while the messenger is visible. -->
<script lang="ts">
  import NoticeBanner from './NoticeBanner.svelte';
  import { openFromNotice } from './desktop';
  import { messengerStore } from '../store.svelte';
</script>

{#if messengerStore.visible}
  <NoticeBanner desk onopen={(chat) => openFromNotice(chat).catch(() => {})} />
{/if}
