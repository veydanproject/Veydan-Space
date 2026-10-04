<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import '@xterm/xterm/css/xterm.css';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import { api } from '$lib/ssh/api';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { parseHostKeyMismatch } from '$lib/core/utils';

  interface KeyboardPromptItem {
    prompt: string;
    echo: boolean;
  }

  interface KeyboardPromptPayload {
    session_id: string;
    name: string;
    instructions: string;
    prompts: KeyboardPromptItem[];
    /** First-connection host key awaiting the user's trust decision. */
    host_key?: string | null;
  }

  interface Props {
    sessionId: string;
    visible?: boolean;
    /** Distance from the bottom of the window to the bottom of the drawer's slot, read when a drag starts. */
    bottomOffset?: () => number;
    onMinimize?: () => void;
    onDisconnect?: () => void;
  }

  let { sessionId, visible = true, bottomOffset = () => 36, onMinimize, onDisconnect }: Props = $props();

  let termEl = $state<HTMLDivElement | undefined>(undefined);
  let term: Terminal | null = null;
  let fitAddon: FitAddon | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let unlistenPrompt: UnlistenFn | null = null;

  // Keyboard-interactive overlay state
  let activePrompt = $state<KeyboardPromptPayload | null>(null);
  let promptInputs = $state<string[]>([]);
  let promptError = $state('');
  let promptSubmitting = $state(false);

  // Drawer size state
  const DEFAULT_VH = 45;
  let drawerVh = $state(DEFAULT_VH);
  let isMaximized = $state(false);
  let isDragging = $state(false);

  let session = $derived(sshStore.sessionById(sessionId));

  // TOFU: new fingerprint from a HOST_KEY_MISMATCH error (null = no mismatch)
  let mismatchFingerprint = $derived(parseHostKeyMismatch(session?.error));
  let trusting = $state(false);

  let drawerHeight = $derived(
    isMaximized ? `calc(100vh - 2.5rem)` : `${drawerVh}vh`
  );

  function startDrag(e: MouseEvent) {
    if (isMaximized) return;
    isDragging = true;
    e.preventDefault();

    const offset = bottomOffset();
    function onMove(ev: MouseEvent) {
      const newHeight = window.innerHeight - ev.clientY - offset;
      const vh = Math.max(20, Math.min(90, (newHeight / window.innerHeight) * 100));
      drawerVh = vh;
    }
    function onUp() {
      isDragging = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      requestAnimationFrame(() => { fitAddon?.fit(); syncSize(); });
    }
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  function toggleMaximize() {
    isMaximized = !isMaximized;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      fitAddon?.fit();
      syncSize();
    }));
  }

  async function answerHostKey(trust: boolean) {
    if (!activePrompt || promptSubmitting) return;
    promptSubmitting = true;
    try {
      await api.ssh.respondPrompt(sessionId, trust ? 'yes' : 'no');
      activePrompt = null;
    } catch (e: unknown) {
      promptError = String(e);
    } finally {
      promptSubmitting = false;
    }
  }

  async function submitPromptResponses() {
    if (!activePrompt || promptSubmitting) return;
    promptSubmitting = true;
    promptError = '';
    try {
      // Send each answer one at a time — backend waits per prompt
      for (const answer of promptInputs) {
        await api.ssh.respondPrompt(sessionId, answer);
      }
      activePrompt = null;
      promptInputs = [];
    } catch (e: unknown) {
      promptError = String(e);
    } finally {
      promptSubmitting = false;
    }
  }

  onMount(async () => {
    if (!termEl) return;

    term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: "'JetBrains Mono Variable', 'JetBrains Mono', 'Fira Code', Menlo, monospace",
      // The terminal viewport is ALWAYS dark, regardless of app theme: remote
      // ANSI output (vim, htop, dotfiles) assumes a dark background.
      // Palette matches the redesign dark theme (JS data — xterm can't read CSS vars)
      theme: {
        background: '#0b0b11',
        foreground: '#eaeaf0',
        cursor: '#8b7bff',
        selectionBackground: 'rgba(139,123,255,0.25)',
      },
      scrollback: 5000,
      allowProposedApi: true,
    });

    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(new WebLinksAddon());
    term.open(termEl);

    term.onData((data) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      api.ssh.sendData(sessionId, bytes).catch(() => {});
    });

    resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(() => { fitAddon?.fit(); syncSize(); });
    });
    resizeObserver.observe(termEl);

    requestAnimationFrame(() => requestAnimationFrame(() => {
      fitAddon?.fit();
      syncSize();
      sshStore.registerDataCallback(sessionId, (bytes: Uint8Array) => {
        term?.write(bytes);
      });
    }));

    // Listen for keyboard-interactive prompts from backend
    unlistenPrompt = await listen<KeyboardPromptPayload>('ssh://keyboard-prompt', (event) => {
      if (event.payload.session_id !== sessionId) return;
      activePrompt = event.payload;
      promptInputs = event.payload.prompts.map(() => '');
      promptError = '';
      // Ensure terminal is visible when prompt arrives
      if (!visible) {
        sshStore.activeTerminalId = sessionId;
      }
    });
  });

  onDestroy(() => {
    unlistenPrompt?.();
    resizeObserver?.disconnect();
    sshStore.unregisterDataCallback(sessionId);
    term?.dispose();
    term = null;
    fitAddon = null;
  });

  // When becoming visible again — refit and repaint
  $effect(() => {
    if (visible && term && fitAddon) {
      requestAnimationFrame(() => requestAnimationFrame(() => {
        fitAddon?.fit();
        term?.refresh(0, term.rows - 1);
        term?.scrollToBottom();
        syncSize();
      }));
    }
  });

  function syncSize() {
    if (!term) return;
    api.ssh.resize(sessionId, term.cols, term.rows).catch(() => {});
  }

  async function disconnect() {
    try {
      await sshStore.disconnect(sessionId);
      await sshStore.removeSession(sessionId);
    } catch {}
    onDisconnect?.();
  }

  async function reconnect() {
    const s = session;
    if (!s) return;
    try {
      const newId = await sshStore.connect(s.connection_id);
      await sshStore.removeSession(sessionId);
      sshStore.activeTerminalId = newId;
      onDisconnect?.();
    } catch {}
  }

  /** Pin the new host key (user confirmed after a mismatch) and reconnect. */
  async function trustAndReconnect() {
    const s = session;
    const fp = mismatchFingerprint;
    if (!s || !fp || trusting) return;
    trusting = true;
    try {
      await api.ssh.connectionTrustFingerprint(s.connection_id, fp);
      await reconnect();
    } catch {
    } finally {
      trusting = false;
    }
  }
</script>

<div class="terminal-drawer" class:hidden={!visible} class:dragging={isDragging} style="height:{drawerHeight}">
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="terminal-wrap">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="drawer-handle" onmousedown={startDrag}>
      <span class="handle-grip"></span>
    </div>
    <div class="terminal-header">
      <span class="terminal-title">
        <Icon name="terminal" size={12} />
        {session?.connection_name ?? sessionId}
        <span class="host-label">{session ? `${session.host}:${session.port}` : ''}</span>
      </span>
      <div class="terminal-status">
        {#if session?.status === 'connected'}
          <span class="dot green"></span>
        {:else if session?.status === 'connecting'}
          <span class="dot yellow"></span>
        {:else if session?.status === 'error' || session?.status === 'disconnected'}
          <span class="dot red"></span>
        {/if}
      </div>
      <div class="header-actions">
        {#if onMinimize}
          <button class="hbtn" onclick={onMinimize} title={$t('ssh_btn_minimize')}>
            <Icon name="minus" size={13} />
          </button>
        {/if}
        <button class="hbtn" onclick={toggleMaximize} title={isMaximized ? $t('window_restore') : $t('window_maximize')}>
          <Icon name={isMaximized ? 'minimize-2' : 'maximize-2'} size={13} />
        </button>
        <button class="hbtn danger" onclick={disconnect} title={$t('ssh_btn_disconnect')}>
          <Icon name="x" size={13} />
        </button>
      </div>
    </div>

    {#if session?.status === 'error' || session?.status === 'disconnected'}
      {#if mismatchFingerprint}
        <!-- Host key changed (TOFU mismatch) — offer to trust the new fingerprint -->
        <div class="disconnected-banner hostkey-banner">
          <Icon name="shield" size={13} />
          <span class="hostkey-text">
            {$t('ssh_hostkey_changed_title')}
            <code class="hostkey-fp">{mismatchFingerprint}</code>
            {$t('ssh_hostkey_changed_hint')}
          </span>
          <button class="btn-primary btn-sm" onclick={trustAndReconnect} disabled={trusting}>
            {trusting ? $t('ssh_connecting') : $t('ssh_hostkey_trust_btn')}
          </button>
        </div>
      {:else}
        <div class="disconnected-banner">
          <Icon name="wifi-off" size={13} />
          {session?.error ? $t('ssh_terminal_error', { msg: session.error }) : $t('ssh_terminal_closed')}
          <button class="btn-primary btn-sm" onclick={reconnect}>{$t('ssh_btn_reconnect')}</button>
        </div>
      {/if}
    {/if}

    <div class="xterm-container" bind:this={termEl}></div>

    {#if activePrompt}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="prompt-overlay">
        <div class="prompt-card">
          {#if activePrompt.host_key}
            <div class="prompt-title">{$t('ssh_hostkey_title')}</div>
            <div class="prompt-instructions">{$t('ssh_hostkey_text')}</div>
            <code class="prompt-fingerprint">{activePrompt.instructions}</code>
            {#if promptError}
              <div class="prompt-error">{promptError}</div>
            {/if}
            <div class="prompt-actions">
              <button class="btn-sm btn-ghost" onclick={() => answerHostKey(false)} disabled={promptSubmitting}>
                {$t('ssh_hostkey_reject')}
              </button>
              <button class="btn-sm btn-primary" onclick={() => answerHostKey(true)} disabled={promptSubmitting}>
                {$t('ssh_hostkey_trust')}
              </button>
            </div>
          {:else}
          {#if activePrompt.name}
            <div class="prompt-title">{activePrompt.name}</div>
          {/if}
          {#if activePrompt.instructions}
            <div class="prompt-instructions">{activePrompt.instructions}</div>
          {/if}
          {#each activePrompt.prompts as p, i}
            <div class="prompt-field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>{p.prompt}</label>
              {#if p.echo}
                <!-- svelte-ignore a11y_autofocus -->
                <input
                  type="text"
                  autofocus={i === 0}
                  bind:value={promptInputs[i]}
                  onkeydown={(e) => e.key === 'Enter' && i === activePrompt!.prompts.length - 1 && submitPromptResponses()}
                />
              {:else}
                <!-- svelte-ignore a11y_autofocus -->
                <input
                  type="password"
                  autofocus={i === 0}
                  bind:value={promptInputs[i]}
                  onkeydown={(e) => e.key === 'Enter' && i === activePrompt!.prompts.length - 1 && submitPromptResponses()}
                />
              {/if}
            </div>
          {/each}
          {#if promptError}
            <div class="prompt-error">{promptError}</div>
          {/if}
          <div class="prompt-actions">
            <button class="btn-sm btn-ghost" onclick={() => { activePrompt = null; promptInputs = []; }} disabled={promptSubmitting}>
              {$t('ssh_btn_cancel')}
            </button>
            <button class="btn-sm btn-primary" onclick={submitPromptResponses} disabled={promptSubmitting}>
              {promptSubmitting ? $t('ssh_connecting') : $t('ssh_btn_send')}
            </button>
          </div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .terminal-drawer {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    pointer-events: auto;
    z-index: 0;
    display: flex;
    flex-direction: column;
    transform: translateY(0);
    transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1),
                opacity 0.2s ease,
                height 0.15s ease;
    opacity: 1;
  }
  .terminal-drawer.hidden {
    transform: translateY(104%);
    opacity: 0;
    pointer-events: none;
  }
  .terminal-drawer.dragging { transition: none; }

  .terminal-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    background: var(--bg);
    border-top: 2px solid var(--border);
    box-shadow: 0 -4px 24px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    position: relative;
  }

  .drawer-handle {
    height: 5px;
    background: var(--bg);
    cursor: ns-resize;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
  }
  .drawer-handle:hover { background: var(--surface-hover); }
  .handle-grip {
    width: 32px;
    height: 2px;
    border-radius: 2px;
    background: var(--border);
    transition: background 0.15s;
  }
  .drawer-handle:hover .handle-grip { background: var(--text-3); }

  .terminal-header {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.2rem var(--sp-2) 0.2rem var(--sp-3);
    background: var(--surface-drawer);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    height: 30px;
  }
  .terminal-title {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: var(--fs-xs);
    color: var(--text-2);
    flex: 1;
    min-width: 0;
  }
  .host-label { color: var(--text-dim); font-size: var(--fs-2xs); }
  .terminal-status { display: flex; align-items: center; }
  .dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
  .dot.green { background: var(--success); }
  .dot.yellow { background: var(--warn-text); }
  .dot.red { background: var(--danger); }

  .header-actions {
    display: flex;
    gap: 0;
    align-items: center;
  }
  .hbtn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 22px;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    border-radius: 3px;
    transition: background 0.12s, color 0.12s;
    padding: 0;
  }
  .hbtn:hover { background: var(--surface-hover); color: var(--text-2); }
  .hbtn.danger:hover { background: var(--danger-bg); color: var(--danger-text); }

  .disconnected-banner {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.3rem var(--sp-3);
    background: var(--danger-bg);
    color: var(--danger-text);
    font-size: var(--fs-xs);
    border-bottom: 1px solid var(--danger-border);
    flex-shrink: 0;
  }
  .disconnected-banner .btn-primary { margin-left: auto; flex-shrink: 0; }

  .hostkey-banner { align-items: flex-start; }
  .hostkey-text { flex: 1; min-width: 0; line-height: 1.5; }
  .hostkey-fp {
    display: block;
    font-family: var(--font-mono);
    font-size: var(--fs-2xs);
    word-break: break-all;
    opacity: 0.9;
  }

  .xterm-container {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    padding: 0.2rem 0.25rem;
    /* Always dark to match the fixed xterm palette — no light seams around the canvas */
    background: #0b0b11;
    color-scheme: dark;
  }
  .xterm-container :global(.xterm) { height: 100%; width: 100%; }
  .xterm-container :global(.xterm-screen) { height: 100%; width: 100%; }
  .xterm-container :global(.xterm-viewport) { overflow-y: auto; }

  /* Keyboard-interactive overlay */
  .prompt-overlay {
    position: absolute;
    inset: 0;
    background: var(--backdrop);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
  }
  .prompt-card {
    background: var(--surface-drawer);
    border: 1px solid var(--border-2);
    border-radius: 8px;
    padding: var(--sp-5) var(--sp-6);
    min-width: 320px;
    max-width: 480px;
    width: 90%;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    box-shadow: var(--shadow-lg);
  }
  .prompt-title {
    font-size: var(--fs-base);
    font-weight: 600;
    color: var(--text);
  }
  .prompt-instructions {
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .prompt-fingerprint {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    word-break: break-all;
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm);
    background: var(--surface-3);
    color: var(--text);
  }
  .prompt-field {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .prompt-field label {
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .prompt-field input {
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 0.45rem 0.6rem;
    color: var(--text);
    font-size: var(--fs-sm);
    outline: none;
    transition: border-color 0.15s;
  }
  .prompt-field input:focus { border-color: var(--accent); }
  .prompt-error {
    font-size: var(--fs-xs);
    color: var(--danger-text);
  }
  .prompt-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-2);
    margin-top: var(--sp-1);
  }
</style>
