<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { ask, confirmTitle } from '$lib/core/ui/confirm.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import PathBar from './PathBar.svelte';
  import FileTable from './FileTable.svelte';
  import PromptDialog from './PromptDialog.svelte';
  import ChmodDialog from './ChmodDialog.svelte';
  import { filesStore } from '$lib/ssh/store/sftp.svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';
  import { api } from '$lib/ssh/api';
  import { t } from '$lib/core/i18n';
  import { parseHostKeyMismatch } from '$lib/core/utils';
  import { explainError } from '$lib/ssh/tor-error';
  import type { FileEntry } from '$lib/ssh/types';

  interface Props {
    index: 0 | 1;
  }

  type SelectOption = { label: string; value: string | null | undefined; disabled?: boolean };

  let { index }: Props = $props();

  let panel = $derived(filesStore.panels[index]);
  let isActive = $derived(filesStore.activePanel === index);
  let opError = $state<string | null>(null);

  // TOFU: new fingerprint from a HOST_KEY_MISMATCH connect error (null = none)
  let mismatchFingerprint = $derived(
    panel.source.kind === 'remote' ? parseHostKeyMismatch(panel.error) : null
  );
  let trusting = $state(false);
  /** The changed key whose plate «Cancel» put away for this attempt: the error of the panel stands instead; a new source or refresh asks again. */
  let dismissedFp = $state<string | null>(null);

  /** Pin the new host key (user confirmed after a mismatch) and reconnect. */
  async function trustHostKey() {
    const fp = mismatchFingerprint;
    if (!fp || panel.source.kind !== 'remote' || trusting) return;
    const connectionId = panel.source.connectionId;
    trusting = true;
    try {
      await run(async () => {
        await api.ssh.connectionTrustFingerprint(connectionId, fp);
        await filesStore.setSource(index, { kind: 'remote', connectionId });
      });
    } finally {
      trusting = false;
    }
  }

  // ── Dialog / menu state ──────────────────────────────────────────────────
  let menu = $state<{ open: boolean; x: number; y: number; entry: FileEntry | null }>({
    open: false, x: 0, y: 0, entry: null,
  });
  let prompt = $state<{
    open: boolean; kind: 'rename' | 'mkdir' | 'newfile'; title: string;
    label: string; value: string; selectTo?: number; target: FileEntry | null;
  }>({ open: false, kind: 'mkdir', title: '', label: '', value: '', target: null });
  let chmod = $state<{ open: boolean; mode: number; label: string }>({ open: false, mode: 0, label: '' });
  let confirmDelete = $state(false);

  let sourceValue = $derived(panel.source.kind === 'local' ? 'local' : panel.source.connectionId);

  let sourceOptions = $derived.by(() => {
    const opts: SelectOption[] = [{ label: $t('files_source_local'), value: 'local' }];
    if (sshStore.connections.length > 0) {
      opts.push({ label: $t('files_source_servers'), value: '__sep__', disabled: true });
      for (const c of sshStore.connections) {
        opts.push({ label: `${c.name} (${c.username}@${c.host})`, value: c.id });
      }
    }
    return opts;
  });

  function onSourceChange(value: string | null | undefined) {
    if (!value || value === '__sep__') return;
    dismissedFp = null;
    filesStore.setSource(index, value === 'local' ? { kind: 'local' } : { kind: 'remote', connectionId: value });
  }

  function onOpen(entry: FileEntry) {
    if (entry.is_dir) filesStore.navigate(index, entry.path);
  }

  function onSelect(entry: FileEntry, mods: { ctrl: boolean; shift: boolean }) {
    filesStore.select(index, entry, mods);
  }

  // Wrap store ops so failures surface as an inline error instead of throwing.
  async function run(fn: () => Promise<void>) {
    opError = null;
    try {
      await fn();
    } catch (e) {
      opError = explainError(e, $t);
    }
  }

  // ── Actions ──────────────────────────────────────────────────────────────
  function askMkdir() {
    prompt = { open: true, kind: 'mkdir', title: $t('files_op_new_folder'), label: $t('files_op_name'), value: '', target: null };
  }
  function askNewFile() {
    prompt = { open: true, kind: 'newfile', title: $t('files_op_new_file'), label: $t('files_op_name'), value: '', target: null };
  }
  function askRename(entry: FileEntry) {
    const dot = entry.is_dir ? -1 : entry.name.lastIndexOf('.');
    prompt = {
      open: true, kind: 'rename', title: $t('files_op_rename'), label: $t('files_op_name'),
      value: entry.name, selectTo: dot > 0 ? dot : entry.name.length, target: entry,
    };
  }
  function onPromptConfirm(value: string) {
    const p = prompt;
    prompt = { ...prompt, open: false };
    if (p.kind === 'mkdir') run(() => filesStore.mkdir(index, value));
    else if (p.kind === 'newfile') run(() => filesStore.createFile(index, value));
    else if (p.kind === 'rename' && p.target) run(() => filesStore.rename(index, p.target!, value));
  }

  function askChmod(entry: FileEntry) {
    const n = panel.selected.length;
    chmod = {
      open: true,
      mode: entry.mode,
      label: n > 1 ? $t('files_selected', { n: String(n) }) : entry.name,
    };
  }
  function onChmodConfirm(mode: number) {
    chmod = { ...chmod, open: false };
    run(() => filesStore.chmodSelected(index, mode));
  }

  /** Asks, then deletes the selection; the keys of the panel wait while it asks. */
  async function askDelete() {
    confirmDelete = true;
    const yes = await ask({
      title: confirmTitle($t('common_delete')),
      message: deleteMessage,
      confirmLabel: $t('common_delete'),
      variant: 'danger',
      icon: 'trash-2',
    });
    confirmDelete = false;
    if (yes) run(() => filesStore.deleteSelected(index));
  }

  // ── Context menu ─────────────────────────────────────────────────────────
  function openRowMenu(entry: FileEntry, x: number, y: number) {
    filesStore.activePanel = index;
    // Right-clicking outside the current selection selects just that entry
    if (!panel.selected.includes(entry.path)) filesStore.select(index, entry, { ctrl: false, shift: false });
    menu = { open: true, x, y, entry };
  }
  function openEmptyMenu(x: number, y: number) {
    filesStore.activePanel = index;
    panel.selected = [];
    menu = { open: true, x, y, entry: null };
  }

  let menuItems = $derived.by<MenuEntry[]>(() => {
    const entry = menu.entry;
    const many = panel.selected.length > 1;
    const items: MenuEntry[] = [];
    if (entry) {
      if (entry.is_dir) items.push({ label: $t('common_open'), icon: 'folder-open', onselect: () => onOpen(entry) });
      items.push({
        label: index === 0 ? $t('files_op_copy_right') : $t('files_op_copy_left'),
        icon: index === 0 ? 'arrow-right' : 'arrow-left', shortcut: 'F5',
        disabled: !filesStore.canCopy(index), onselect: () => run(() => filesStore.copyToOtherPanel(index)),
      });
      items.push({
        label: $t('files_op_move'), icon: 'file-arrow-right', shortcut: 'F6',
        disabled: !canMove(), onselect: () => run(() => filesStore.moveToOtherPanel(index)),
      });
      items.push({ type: 'separator' });
      items.push({
        label: $t('files_op_rename'), icon: 'pencil', shortcut: 'F2',
        disabled: many, onselect: () => askRename(entry),
      });
      items.push({ label: $t('files_op_chmod'), icon: 'file-text-shield', onselect: () => askChmod(entry) });
      items.push({
        label: $t('files_op_copy_path'), icon: 'copy', disabled: many,
        onselect: () => filesStore.clipboardPath(entry),
      });
      items.push({ type: 'separator' });
    }
    items.push({ label: $t('files_op_new_folder'), icon: 'folder-plus', shortcut: 'F7', onselect: askMkdir });
    items.push({ label: $t('files_op_new_file'), icon: 'file', onselect: askNewFile });
    if (entry) {
      items.push({ type: 'separator' });
      items.push({
        label: many ? $t('files_op_delete_n', { n: String(panel.selected.length) }) : $t('common_delete'),
        icon: 'trash-2', shortcut: 'Del', danger: true, onselect: () => void askDelete(),
      });
    }
    return items;
  });

  function canMove(): boolean {
    if (panel.selected.length === 0) return false;
    const other = filesStore.panels[index === 0 ? 1 : 0];
    const same = panel.source.kind === 'local'
      ? other.source.kind === 'local'
      : other.source.kind === 'remote' && other.source.connectionId === panel.source.connectionId;
    return same || filesStore.canCopy(index);
  }

  // ── Keyboard (only when this panel is active) ────────────────────────────
  function onKeydown(e: KeyboardEvent) {
    if (!isActive) return;
    const el = e.target as HTMLElement | null;
    if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) return;
    if (prompt.open || chmod.open || confirmDelete || menu.open) return;

    const sel = panel.selected;
    const one = sel.length === 1 ? panel.entries.find((x) => x.path === sel[0]) : null;
    switch (e.key) {
      case 'F2':
        if (one) { e.preventDefault(); askRename(one); }
        break;
      case 'F6':
        if (canMove()) { e.preventDefault(); run(() => filesStore.moveToOtherPanel(index)); }
        break;
      case 'F7':
        e.preventDefault(); askMkdir();
        break;
      case 'Delete':
      case 'F8':
        if (sel.length > 0) { e.preventDefault(); void askDelete(); }
        break;
    }
  }

  let deleteMessage = $derived(
    panel.selected.length > 1
      ? $t('files_delete_confirm_n', { n: String(panel.selected.length) })
      : $t('files_delete_confirm', { name: panel.entries.find((e) => e.path === panel.selected[0])?.name ?? '' })
  );
</script>

<svelte:window onkeydown={onKeydown} />

<section
  class="on-glass file-panel"
  class:focused={isActive}
  role="button"
  tabindex="-1"
  onclick={() => (filesStore.activePanel = index)}
  onfocusin={() => (filesStore.activePanel = index)}
  onkeydown={(e) => {
    if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
      filesStore.activePanel = index;
    }
  }}
>
  <div class="panel-source">
    <CustomSelect options={sourceOptions} value={sourceValue} onchange={onSourceChange} />
  </div>

  <PathBar
    path={panel.path}
    showHidden={panel.showHidden}
    onnavigate={(p) => filesStore.navigate(index, p)}
    onup={() => filesStore.up(index)}
    onrefresh={() => { dismissedFp = null; filesStore.refresh(index); }}
    ontogglehidden={() => filesStore.toggleHidden(index)}
  />

  <div class="panel-toolbar">
    <button
      class="btn btn-ghost btn-sm"
      disabled={!filesStore.canCopy(index)}
      title={`${$t('files_copy_hint')} (F5)`}
      onclick={() => run(() => filesStore.copyToOtherPanel(index))}
    >
      {#if index === 0}
        {$t('common_copy')} <Icon name="arrow-right" size={13} />
      {:else}
        <Icon name="arrow-left" size={13} /> {$t('common_copy')}
      {/if}
    </button>
    <span class="toolbar-gap"></span>
    <button class="icon-btn" title={`${$t('files_op_new_folder')} (F7)`} onclick={askMkdir}>
      <Icon name="folder-plus" size={14} />
    </button>
    <button class="icon-btn" title={$t('files_op_new_file')} onclick={askNewFile}>
      <Icon name="file" size={14} />
    </button>
    <button
      class="icon-btn"
      title={`${$t('files_op_rename')} (F2)`}
      disabled={panel.selected.length !== 1}
      onclick={() => { const e = panel.entries.find((x) => x.path === panel.selected[0]); if (e) askRename(e); }}
    >
      <Icon name="pencil" size={14} />
    </button>
    <button
      class="icon-btn"
      title={$t('files_op_chmod')}
      disabled={panel.selected.length === 0}
      onclick={() => { const e = panel.entries.find((x) => x.path === panel.selected[0]); if (e) askChmod(e); }}
    >
      <Icon name="file-text-shield" size={14} />
    </button>
    <button
      class="icon-btn danger-soft"
      title={`${$t('common_delete')} (Del)`}
      disabled={panel.selected.length === 0}
      onclick={askDelete}
    >
      <Icon name="trash-2" size={18} />
    </button>
    {#if panel.selected.length > 1}
      <span class="sel-count">{$t('files_selected', { n: String(panel.selected.length) })}</span>
    {/if}
  </div>

  {#if opError}
    <div class="plate"><Notice tone="danger">{opError}</Notice></div>
  {:else if mismatchFingerprint && dismissedFp !== mismatchFingerprint}
    <!-- Host key changed (TOFU mismatch): the one plate of a server's key, as in the terminal (SSHTerminal). -->
    <div class="plate">
      <Notice tone="warn" role="alert">
        <b>{$t('ssh_hostkey_changed_title')}</b>
        <code class="hostkey-fp">{mismatchFingerprint}</code>
        {$t('ssh_hostkey_changed_hint')}
        {#snippet action()}
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (dismissedFp = mismatchFingerprint)} disabled={trusting}>{$t('common_cancel')}</button>
          <button class="btn btn-danger-soft btn-sm" type="button" onclick={trustHostKey} disabled={trusting}>
            {trusting ? '…' : $t('ssh_hostkey_trust_btn')}
          </button>
        {/snippet}
      </Notice>
    </div>
  {:else if panel.error}
    <div class="plate"><Notice tone="danger">{panel.error}</Notice></div>
  {/if}

  {#if panel.loading}
    <div class="panel-state"><span class="spinner"></span></div>
  {:else}
    <FileTable
      entries={filesStore.visibleEntries(index)}
      selected={panel.selected}
      sortKey={panel.sortKey}
      sortDir={panel.sortDir}
      onsort={(key) => filesStore.setSort(index, key)}
      onopen={onOpen}
      onselect={onSelect}
      oncontextmenu={openRowMenu}
      onemptycontextmenu={openEmptyMenu}
    />
  {/if}
</section>

<ContextMenu
  bind:open={menu.open}
  x={menu.x}
  y={menu.y}
  items={menuItems}
  onclose={() => (menu.open = false)}
>
  {#snippet header()}
    {#if menu.entry}{menu.entry.name}{:else}{panel.path}{/if}
  {/snippet}
</ContextMenu>

<PromptDialog
  bind:open={prompt.open}
  title={prompt.title}
  label={prompt.label}
  value={prompt.value}
  selectTo={prompt.selectTo}
  onconfirm={onPromptConfirm}
  oncancel={() => (prompt.open = false)}
/>

<ChmodDialog
  bind:open={chmod.open}
  targetLabel={chmod.label}
  mode={chmod.mode}
  onconfirm={onChmodConfirm}
  oncancel={() => (chmod.open = false)}
/>

<style>
  .file-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
    min-height: 0;
    padding: var(--sp-3);
    background: var(--pane);
    border: 1px solid var(--pane-border);
    border-radius: var(--radius-pane);
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
  }
  .file-panel.focused {
    border-color: var(--accent-border);
  }

  .panel-source {
    flex-shrink: 0;
  }

  .panel-toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }
  .toolbar-gap { flex: 1; }
  .sel-count {
    color: var(--text-3);
    font-size: var(--fs-xs);
  }

  .panel-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-3);
  }

  .plate { padding: var(--sp-2) var(--sp-3) 0; flex-shrink: 0; }
  .hostkey-fp {
    display: block;
    margin: var(--sp-1) 0;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    word-break: break-all;
  }
</style>
