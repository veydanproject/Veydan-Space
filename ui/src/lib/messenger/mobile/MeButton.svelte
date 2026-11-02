<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- My avatar in the header of the phone's tabs: my profile and the settings of the chat. A product made of this module alone (Veydan Chat) has no Settings in its bar: the shell gives it the app's settings to open here instead (context `app-settings`). -->
<script lang="ts">
  import { getContext } from 'svelte';
  import type { Readable } from 'svelte/store';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Avatar from '../contacts/Avatar.svelte';
  import { profileLabel } from '../api';
  import { messengerStore } from '../store.svelte';
  import { BASE } from './routes';

  const app = getContext<{ href: string; label: Readable<string> } | undefined>('app-settings');
  const appLabel = app?.label;
</script>

<button class="me" onclick={() => goto(app?.href ?? `${BASE}/settings`)} aria-label={$appLabel ?? $t('msg_settings_title_full')}>
  <Avatar url={messengerStore.ownProfile?.picture ?? null} label={profileLabel(messengerStore.ownProfile) || $t('msg_you')} seed={messengerStore.identity?.pubkey ?? null} size={34} />
</button>

<style>
  .me { display: inline-flex; padding: 5px; margin-right: 2px; --hit-r: calc(var(--edge-r, 0px) + 2px); border: none; border-radius: 50%; background: none; cursor: pointer; }
  .me:active { opacity: 0.8; }
</style>
