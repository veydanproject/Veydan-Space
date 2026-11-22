<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The filter by workspace of the phone's password and 2FA lists, in «⋮» of
  their head (design 16): «Показать все» and the workspaces, the chosen one
  ticked — the computer's ws-chips (PasswordDrawer) as a sheet. `inWorkspace`
  is the computer's test (the stores' byWorkspace): the workspace's own tag, or
  a profile of it.
-->
<script lang="ts" module>
  import { directory } from '$lib/core/directory';

  /** An entry of the workspace: tagged with it or with one of its profiles. */
  export function inWorkspace(tags: string[], workspaceId: string): boolean {
    if (tags.includes(`workspace:${workspaceId}`)) return true;
    const profiles = new Set(
      directory.list('profile').filter((p) => p.parent === `workspace:${workspaceId}`).map((p) => `profile:${p.id}`),
    );
    return tags.some((tag) => profiles.has(tag));
  }
</script>

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { t } from '$lib/core/mobile/i18n';

  interface Props {
    open: boolean;
    /** The list's title, the sheet's head. */
    title: string;
    /** The chosen workspace; null shows every one. */
    active: string | null;
    onpick: (workspaceId: string | null) => void;
    onclose: () => void;
  }

  let { open, title, active, onpick, onclose }: Props = $props();
</script>

<BottomSheet {open} {title} {onclose}>
  <div class="m-list" role="radiogroup" aria-label={title}>
    <button type="button" class="list-row dense" role="radio" aria-checked={active === null} onclick={() => onpick(null)}>
      <Icon name="layers" size={20} /><span class="list-row-label">{$t('totp_show_all')}</span>
      {#if active === null}<span class="tick"><Icon name="check" size={20} /></span>{/if}
    </button>
    {#each directory.list('workspace') as ws (ws.id)}
      <button type="button" class="list-row dense" role="radio" aria-checked={active === ws.id} onclick={() => onpick(ws.id)}>
        <span class="ws tinted-text" style:--chip={ws.color || undefined}><Icon name="layers" size={20} /></span>
        <span class="list-row-label">{ws.name}</span>
        {#if active === ws.id}<span class="tick"><Icon name="check" size={20} /></span>{/if}
      </button>
    {/each}
  </div>
</BottomSheet>

<style>
  .ws, .tick { display: inline-flex; }
  .tick { color: var(--accent-text); }
</style>
