<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  let activeTerminalId = $derived(sshStore.activeTerminalId);

  function setActive(id: string | null) {
    sshStore.activeTerminalId = id;
  }

  function statusColor(status: string): string {
    if (status === 'connected') return 'var(--success)';
    if (status === 'connecting') return 'var(--warn-text)';
    return 'var(--danger)';
  }
</script>

{#if sshStore.sessions.length > 0}
  <div class="ssh-bar">
    <span class="ssh-label">
      <Icon name="terminal" size={12} />
      {$t('ssh_bar_label')}
    </span>
    <div class="ssh-sessions">
      {#each sshStore.sessions as s (s.session_id)}
        <button
          class="ssh-chip"
          class:error={s.status === 'error' || s.status === 'disconnected'}
          class:active={activeTerminalId === s.session_id}
          onclick={() => setActive(activeTerminalId === s.session_id ? null : s.session_id)}
          title={`${s.connection_name} — ${s.status}${s.error ? ': ' + s.error : ''}`}
        >
          <span class="chip-dot" style="background:{statusColor(s.status)}"></span>
          <span class="chip-name">{s.connection_name}</span>
          <span class="chip-host">{s.host}</span>
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .ssh-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.3rem var(--sp-4);
    background: var(--bg-2);
    border-top: 1px solid var(--border);
    overflow-x: auto;
    flex-shrink: 0;
    height: var(--bar-h);
  }
  .ssh-label {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--fs-2xs);
    text-transform: uppercase;
    color: var(--text-3);
    font-weight: 600;
    flex-shrink: 0;
    letter-spacing: 0.05em;
  }
  .ssh-sessions { display: flex; gap: 0.35rem; align-items: center; }
  .ssh-chip {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.2rem 0.55rem;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    border: 1px solid var(--border);
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    transition: all var(--dur-fast);
    white-space: nowrap;
  }
  .ssh-chip:hover { border-color: var(--border-2); }
  .ssh-chip.active {
    border-color: var(--accent-tint-border);
    background: var(--accent-tint);
    color: var(--accent-text-2);
  }
  .ssh-chip.error { border-color: var(--danger-border); background: var(--danger-bg); color: var(--danger-text); }
  .chip-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
  .chip-name { font-weight: var(--fw-medium); }
  .chip-host { color: var(--text-3); font-size: var(--fs-2xs); font-family: var(--font-mono); }
</style>
