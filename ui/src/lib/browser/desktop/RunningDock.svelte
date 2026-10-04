<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The dock at the bottom of the window: the browser profiles running right now. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import type { Profile } from '$lib/browser/types';
  import { profilesStore } from '$lib/browser/store/profiles.svelte';
  import { runningStore } from '$lib/browser/store/running.svelte';

  let runningProfiles = $derived(profilesStore.list.filter((p) => runningStore.ids.includes(p.id)));

  function getWorkspaceHref(p: Profile) {
    return p.workspace_id ? `/workspace/${p.workspace_id}` : '/';
  }
</script>

{#if runningProfiles.length > 0}
  <div class="dock">
    <span class="dock-label">
      <span class="dock-dot"></span>
      {runningProfiles.length} running
    </span>
    <div class="dock-sessions">
      {#each runningProfiles as p (p.id)}
        <a href={getWorkspaceHref(p)} class="dock-item" title={p.name}>
          <Icon name="globe" size={13} />
          <span class="dock-name">{p.name}</span>
        </a>
      {/each}
    </div>
  </div>
{/if}

<style>
  .dock {
    height: var(--dock-h);
    background: var(--bg-2);
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0 1rem;
    flex-shrink: 0;
    overflow: hidden;
  }

  .dock-label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--fs-2xs);
    color: var(--text-2);
    white-space: nowrap;
  }

  .dock-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success);
    animation: pulse 2s infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .dock-sessions {
    display: flex;
    gap: 0.35rem;
    overflow-x: auto;
    flex: 1;
  }

  .dock-sessions::-webkit-scrollbar { display: none; }

  .dock-item {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.2rem 0.55rem;
    background: var(--success-bg);
    border: 1px solid color-mix(in srgb, var(--success) 25%, var(--border));
    border-radius: 999px;
    font-size: var(--fs-2xs);
    color: var(--success-text);
    white-space: nowrap;
    text-decoration: none;
    transition: filter 0.15s;
    flex-shrink: 0;
  }
  .dock-item:hover { filter: brightness(1.15); }
  .dock-name { max-width: 100px; overflow: hidden; text-overflow: ellipsis; }
</style>
