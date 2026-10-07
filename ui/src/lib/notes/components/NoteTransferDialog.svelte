<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api } from '$lib/notes/api';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { t, locale, countKey } from '$lib/core/i18n';

  export type TransferMode = 'export' | 'import';

  interface Props {
    mode: TransferMode | null;
    /** Notes to export */
    exportIds?: string[];
    /** Bindings applied to imported notes without frontmatter */
    importBindings?: string[];
    onclose: () => void;
  }

  let { mode, exportIds = [], importBindings = [], onclose }: Props = $props();

  let busy = $state(false);
  let result = $state<string | null>(null);
  let error = $state<string | null>(null);
  /** Passphrase for `.age` export/import; empty means plain */
  let password = $state('');

  $effect(() => { mode; result = null; error = null; password = ''; });

  async function run(task: () => Promise<string | null>) {
    busy = true;
    error = null;
    try {
      result = await task();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const exportTo = (asZip: boolean) => run(async () => {
    const { open, save } = await import('@tauri-apps/plugin-dialog');
    const dest = asZip
      ? await save({ defaultPath: 'notes.zip', filters: [{ name: 'ZIP', extensions: ['zip'] }] })
      : await open({ directory: true, multiple: false });
    if (!dest) return null;
    const r = await api.notes.export(exportIds, dest, asZip);
    return $t(countKey('notes_export_done', r.count, $locale), { n: String(r.count), path: r.path });
  });

  const exportEncrypted = () => run(async () => {
    const { save } = await import('@tauri-apps/plugin-dialog');
    const dest = await save({ defaultPath: 'notes.zip.age', filters: [{ name: 'age', extensions: ['age'] }] });
    if (!dest) return null;
    const r = await api.notes.export(exportIds, dest, true, password);
    return $t(countKey('notes_export_done', r.count, $locale), { n: String(r.count), path: r.path });
  });

  const importFrom = (directory: boolean) => run(async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const picked = await open(
      directory
        ? { directory: true, multiple: false }
        : { multiple: true, filters: [{ name: $t('notes_title'), extensions: ['md', 'markdown', 'txt', 'zip', 'age'] }] }
    );
    if (!picked) return null;
    const paths = Array.isArray(picked) ? picked : [picked];
    const ids = await api.notes.import(paths, importBindings, password || undefined);
    await notesStore.refresh();
    return $t(countKey('notes_import_done', ids.length, $locale), { n: String(ids.length) });
  });
</script>

<Dialog
  open={mode !== null}
  width="420px"
  title={mode === 'export' ? $t('notes_export_title') : $t('notes_import_title')}
  {onclose}
>
  <div class="body">
    {#if mode === 'export'}
      <p class="hint">{$t(countKey('notes_export_hint', exportIds.length, $locale), { n: String(exportIds.length) })}</p>
      <div class="choices">
        <button class="btn btn-primary btn-sm" disabled={busy || exportIds.length === 0} onclick={() => exportTo(false)}>
          <Icon name="folder-open" size={13} /> {$t('notes_export_dir')}
        </button>
        <button class="btn btn-ghost btn-sm" disabled={busy || exportIds.length === 0} onclick={() => exportTo(true)}>
          <Icon name="download" size={13} /> {$t('notes_export_zip')}
        </button>
      </div>
      <div class="encrypted">
        <input type="password" bind:value={password} placeholder={$t('notes_export_password')} autocomplete="off" />
        <button class="btn btn-ghost btn-sm" disabled={busy || exportIds.length === 0 || password.length < 4} onclick={exportEncrypted}>
          <Icon name="lock" size={13} /> {$t('notes_export_encrypted')}
        </button>
      </div>
    {:else}
      <p class="hint">{$t('notes_import_hint')}</p>
      <input type="password" bind:value={password} placeholder={$t('notes_import_password')} autocomplete="off" />
      <div class="choices">
        <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => importFrom(false)}>
          <Icon name="file-text" size={13} /> {$t('notes_import_files')}
        </button>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => importFrom(true)}>
          <Icon name="folder-open" size={13} /> {$t('notes_import_dir')}
        </button>
      </div>
    {/if}
    {#if result}<p class="result ok">{result}</p>{/if}
    {#if error}<p class="result err">{error}</p>{/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .choices { display: flex; gap: var(--sp-2); }
  .choices .btn { flex: 1; justify-content: center; }
  /* The passphrase has a row of its own: its placeholder is read whole. */
  .encrypted { display: flex; flex-direction: column; align-items: flex-end; gap: var(--sp-2); }
  .encrypted input { width: 100%; min-width: 0; }
  .result { margin: 0; font-size: var(--fs-xs); word-break: break-all; }
  .result.ok { color: var(--success); }
  .result.err { color: var(--danger); }
</style>
