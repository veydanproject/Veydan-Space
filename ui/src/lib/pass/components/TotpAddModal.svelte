<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/pass/api';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { directory } from '$lib/core/directory';
  import type { TotpEntry, TotpPreview } from '$lib/pass/types';
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import TermHint from '$lib/core/ui/TermHint.svelte';
  import { formatError } from '$lib/core/utils';
  import { isEntityKind, parseBinding } from '$lib/core/bindings';
  import { isSystemTag, mergeTags, systemTags, userLabels } from '$lib/pass/totp-tags';
  import { notesLinkedToTotp, syncTotpNotes } from '$lib/pass/totp-notes';
  import TotpNoteLinks from '$lib/pass/components/TotpNoteLinks.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';

  interface Props {
    initialTags?: string[];
    /** Set to edit name, issuer, and tags. Secret stays unchanged. */
    entry?: TotpEntry | null;
    onclose: () => void;
    onadded?: () => void;
  }

  const TAG_COLORS = [
    // The kit's presets (N.colors); the colour is the user's, never the kind.
    '#8b7bff', '#5ab0f0', '#59c28a', '#e8a33d', '#e66b6b', '#d58ad6',
  ];

  let { initialTags = [], entry = null, onclose, onadded }: Props = $props();
  const editing = $derived(entry !== null);

  let tab = $state<'manual' | 'qr'>('manual');

  // Form fields. Seeded once so a parent re-render does not wipe what the user typed.
  let name = $state('');
  let issuer = $state('');
  let secret = $state('');
  let algorithm = $state('SHA1');
  let digits = $state(6);
  let period = $state(30);
  let query = $state('');
  let suggest = $state(false);
  let pickedColor = $state(TAG_COLORS[0]);
  let labelNames = $state<string[]>([]);
  let extraBindings = $state<string[]>([]);
  let seeded = false;

  $effect.pre(() => {
    if (seeded) return;
    const source = entry?.tags ?? initialTags;
    name = entry?.name ?? '';
    issuer = entry?.issuer ?? '';
    labelNames = userLabels(source);
    extraBindings = entry ? systemTags(entry.tags) : [];
    seeded = true;
  });

  const lockedBindings = $derived(entry ? [] : initialTags.filter(isSystemTag));

  onMount(() => {
    void directory.ensureLoaded();
  });

  const chosenBindings = $derived(new Set([...lockedBindings, ...extraBindings]));
  const q = $derived(query.trim().toLowerCase());
  const tagHits = $derived(
    directory.list('note_tag')
      .filter((tag) => !labelNames.includes(tag.name) && tag.name.toLowerCase().includes(q))
      .slice(0, 6),
  );
  const profileHits = $derived(
    q
      ? directory.list('profile')
          .filter((profile) => !chosenBindings.has(`profile:${profile.id}`) && profile.name.toLowerCase().includes(q))
          .slice(0, 4)
      : [],
  );
  const workspaceHits = $derived(
    q
      ? directory.list('workspace')
          .filter((ws) => !chosenBindings.has(`workspace:${ws.id}`) && ws.name.toLowerCase().includes(q))
          .slice(0, 4)
      : [],
  );
  const canCreate = $derived(
    q.length > 0 && !directory.list('note_tag').some((tag) => tag.name.toLowerCase() === q),
  );

  // QR state
  let qrPreview = $state<TotpPreview | null>(null);
  let qrUri = $state('');
  let qrError = $state('');
  let qrLoading = $state(false);
  let fileInput: HTMLInputElement | null = $state(null);

  let saving = $state(false);
  let error = $state('');
  // A required field's error stands under that field; any other error is the plate of the form.
  const nameErr = $derived(!!error && error === $t('totp_error_name'));
  const secretErr = $derived(!!error && tab === 'manual' && error === $t('totp_error_secret'));
  let noteIds = $state<string[]>([]);
  let noteSeed = $state<string[]>([]);
  let notesSeeded = false;

  const noteOptions = $derived(directory.list('note').map((n) => ({ id: n.id, title: n.name })));

  $effect(() => {
    if (notesSeeded || !entry || !directory.has('note') || noteOptions.length === 0) return;
    const ids = notesLinkedToTotp(entry.id);
    noteIds = ids;
    noteSeed = [...ids];
    notesSeeded = true;
  });

  function bindingKind(tag: string): string {
    return parseBinding(tag)?.kind ?? 'workspace';
  }

  function bindingLabel(tag: string): string {
    const parsed = parseBinding(tag);
    if (!parsed) return tag;
    // The entity's name; a kind without an owner here as 10.3 shows it (its label, else the kind and a short id).
    if (isEntityKind(parsed.kind)) return directory.name(parsed.kind, parsed.value, $t);
    return parsed.value;
  }

  /** A workspace in the colour the user gave it; a profile neutral (its mark tells the kind, never a colour). */
  function bindingColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    return parsed?.kind === 'workspace' ? directory.color('workspace', parsed.value) : undefined;
  }

  function labelColor(name: string): string | undefined {
    return directory.summary('note_tag', name)?.color;
  }

  function addLabelName(name: string) {
    const value = name.trim();
    if (!value || isSystemTag(value) || labelNames.includes(value)) return;
    labelNames = [...labelNames, value];
    query = '';
  }

  function addBinding(tag: string) {
    if (!isSystemTag(tag) || chosenBindings.has(tag)) return;
    extraBindings = [...extraBindings, tag];
    query = '';
  }

  function removeLabel(label: string) {
    labelNames = labelNames.filter((item) => item !== label);
  }

  function removeBinding(tag: string) {
    extraBindings = extraBindings.filter((item) => item !== tag);
  }

  let pendingCommit: Promise<void> | null = null;

  function uniqueNamed(list: { id: string; name: string }[], value: string) {
    const hits = list.filter((item) => item.name.toLowerCase() === value.toLowerCase());
    return hits.length === 1 ? hits[0] : null;
  }

  /** Turn one typed token into a label or a profile/workspace binding. */
  async function addToken(value: string) {
    if (isSystemTag(value)) {
      addBinding(value);
      return;
    }
    const existing = directory.list('note_tag').find((tag) => tag.name.toLowerCase() === value.toLowerCase());
    if (existing) {
      addLabelName(existing.name);
      return;
    }
    const profile = uniqueNamed(directory.list('profile'), value);
    if (profile) {
      addBinding(`profile:${profile.id}`);
      return;
    }
    const workspace = uniqueNamed(directory.list('workspace'), value);
    if (workspace) {
      addBinding(`workspace:${workspace.id}`);
      return;
    }
    // The owner of the labels makes a new one; without it the label is free text.
    const created = await directory.get('note_tag')?.create?.(value, pickedColor);
    addLabelName(created?.name ?? value);
  }

  /** Apply the tag field. Empty input succeeds. */
  async function commitQuery(): Promise<boolean> {
    if (pendingCommit) {
      try {
        await pendingCommit;
      } catch {
        return false;
      }
      return !error;
    }
    const parts = query.split(',').map((part) => part.trim()).filter(Boolean);
    if (parts.length === 0) return true;
    query = '';
    const job = (async () => {
      await directory.ensureLoaded();
      for (const part of parts) await addToken(part);
    })();
    pendingCommit = job;
    try {
      await job;
      return true;
    } catch (err) {
      error = formatError(err);
      return false;
    } finally {
      if (pendingCommit === job) pendingCommit = null;
    }
  }

  async function handleQrFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    qrError = '';
    qrPreview = null;
    qrUri = '';
    qrLoading = true;

    try {
      const bitmap = await createImageBitmap(file);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width;
      canvas.height = bitmap.height;
      const ctx = canvas.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);

      const jsQR = (await import('jsqr')).default;
      const result = jsQR(imageData.data, imageData.width, imageData.height);

      if (!result) {
        qrError = $t('totp_qr_not_found');
        return;
      }

      qrUri = result.data;
      qrPreview = await api.totp.previewUri(qrUri);

      name = qrPreview.name;
      issuer = qrPreview.issuer ?? '';
      algorithm = qrPreview.algorithm;
      digits = qrPreview.digits;
      period = qrPreview.period;
    } catch (err) {
      qrError = formatError(err);
    } finally {
      qrLoading = false;
      if (fileInput) fileInput.value = '';
    }
  }

  async function save() {
    error = '';
    if (!name.trim()) { error = $t('totp_error_name'); return; }
    if (!editing && tab === 'manual' && !secret.trim()) { error = $t('totp_error_secret'); return; }
    if (!editing && tab === 'qr' && !qrUri) { error = $t('totp_error_secret'); return; }

    saving = true;
    try {
      if (!(await commitQuery())) return;
      const tags = mergeTags(lockedBindings, extraBindings, labelNames);
      const saved = entry
        ? await api.totp.update(entry.id, { name: name.trim(), issuer: issuer.trim() || null, tags })
        : tab === 'qr'
          ? await api.totp.add({ name: name.trim(), issuer: issuer.trim() || null, uri: qrUri, tags })
          : await api.totp.add({ name: name.trim(), issuer: issuer.trim() || null, secret: secret.trim(), algorithm, digits, period, tags });
      await syncTotpNotes(saved.id, noteIds, noteSeed);
      await totpStore.refresh();
      onadded?.();
      onclose();
    } catch (err) {
      error = formatError(err);
    } finally {
      saving = false;
    }
  }
</script>

<Dialog open title={editing ? $t('totp_edit_title') : $t('totp_add_title')} width="460px" {onclose}>

    {#if !editing}
    <div class="tab-bar">
      <button class="tab" class:active={tab === 'manual'} onclick={() => { tab = 'manual'; qrPreview = null; qrUri = ''; qrError = ''; }}>
        <Icon name="keyboard" size={13} /> {$t('totp_tab_manual')}
      </button>
      <button class="tab" class:active={tab === 'qr'} onclick={() => { tab = 'qr'; }}>
        <Icon name="scan" size={13} /> {$t('totp_tab_qr')}
      </button>
    </div>
    {/if}

    <div class="totp-form">
      {#if !editing && tab === 'qr'}
        <div class="qr-section">
          <p class="hint">{$t('totp_qr_hint')}</p>
          <button type="button" class="btn btn-ghost" onclick={() => fileInput?.click()} disabled={qrLoading}>
            <Icon name="image" size={14} />
            {$t('totp_qr_select')}
          </button>
          <input bind:this={fileInput} type="file" accept="image/*" style="display:none" onchange={handleQrFile} />
          {#if qrLoading}<span class="hint">{$t('loading')}</span>{/if}
          {#if qrError}<div class="error-msg">{qrError}</div>{/if}
          {#if qrPreview}
            <div class="qr-detected">
              <span style="color:var(--success);display:flex"><Icon name="check" size={14} /></span>
              {$t('totp_qr_detected')}
              <span class="masked mono">{$t('totp_qr_secret_masked')} {qrPreview.secret_masked}</span>
            </div>
          {/if}
        </div>
      {/if}

      <div class="form-group">
        <label for="totp-name">{$t('totp_field_name')}</label>
        <input id="totp-name" type="text" bind:value={name} placeholder={$t('totp_field_name_placeholder')} aria-invalid={nameErr} />
        {#if nameErr}<div class="error-msg">{error}</div>{/if}
      </div>

      <div class="form-group">
        <label for="totp-issuer">{$t('totp_field_issuer')}</label>
        <input id="totp-issuer" type="text" bind:value={issuer} placeholder={$t('totp_field_issuer_placeholder')} />
      </div>

      {#if !editing && tab === 'manual'}
        <div class="form-group">
          <div class="label-row">
            <label for="totp-secret">{$t('totp_field_secret')}</label>
            <TermHint term={$t('totp_field_secret')}>{$t('totp_hint_secret')}</TermHint>
          </div>
          <input id="totp-secret" class="mono" type="text" bind:value={secret} placeholder={$t('totp_field_secret_placeholder')} autocomplete="off" aria-invalid={secretErr} />
          {#if secretErr}<div class="error-msg">{error}</div>{/if}
        </div>
      {/if}

      {#if !editing}
      <div class="form-row">
        <div class="form-group">
          <div class="label-row">
            <label for="totp-algo">{$t('totp_field_algorithm')}</label>
            <TermHint term={$t('totp_field_algorithm')}>{$t('totp_hint_algorithm')}</TermHint>
          </div>
          <CustomSelect
            id="totp-algo"
            options={[
              { value: 'SHA1', label: $t('totp_algo_default') },
              { value: 'SHA256', label: 'SHA256' },
              { value: 'SHA512', label: 'SHA512' },
            ]}
            bind:value={algorithm}
          />
        </div>
        <div class="form-group">
          <div class="label-row">
            <label for="totp-digits">{$t('totp_field_digits')}</label>
            <TermHint term={$t('totp_field_digits')}>{$t('totp_hint_digits')}</TermHint>
          </div>
          <CustomSelect
            id="totp-digits"
            options={[{ value: '6', label: '6' }, { value: '8', label: '8' }]}
            value={String(digits)}
            onchange={(v) => (digits = Number(v))}
          />
        </div>
        <div class="form-group">
          <div class="label-row">
            <label for="totp-period">{$t('totp_field_period')}</label>
            <TermHint term={$t('totp_field_period')}>{$t('totp_hint_period')}</TermHint>
          </div>
          <CustomSelect
            id="totp-period"
            options={[{ value: '30', label: '30' }, { value: '60', label: '60' }]}
            value={String(period)}
            onchange={(v) => (period = Number(v))}
          />
        </div>
      </div>
      {/if}

      <div class="form-group">
        <label for="totp-label-input">{$t('totp_field_tags')}</label>
        {#if lockedBindings.length > 0 || extraBindings.length > 0 || labelNames.length > 0}
          <div class="label-chips">
            {#each lockedBindings as tag (tag)}
              <span class="context-chip tinted" style:--chip={bindingColor(tag)}>
                <ChipMark kind={bindingKind(tag)} />
                {bindingLabel(tag)}
              </span>
            {/each}
            {#each extraBindings as tag (tag)}
              <span class="context-chip tinted" style:--chip={bindingColor(tag)}>
                <ChipMark kind={bindingKind(tag)} />
                {bindingLabel(tag)}
                <button type="button" onclick={() => removeBinding(tag)} aria-label={$t('pass_remove')} title={$t('pass_remove')}>×</button>
              </span>
            {/each}
            {#each labelNames as label (label)}
              <span class="label-chip tinted" style:--chip={labelColor(label)}>
                <ChipMark kind="tag" />
                {label}
                <button type="button" onclick={() => removeLabel(label)} aria-label={$t('pass_remove')} title={$t('pass_remove')}>×</button>
              </span>
            {/each}
          </div>
        {/if}
        <input
          id="totp-label-input"
          type="text"
          bind:value={query}
          placeholder={$t('notes_tags_placeholder')}
          onfocus={() => (suggest = true)}
          onblur={() => (suggest = false)}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); void commitQuery(); }
          }}
        />
        {#if suggest && (tagHits.length > 0 || profileHits.length > 0 || workspaceHits.length > 0 || canCreate)}
          <!-- mousedown keeps the field focused so the list is not dismissed before the click -->
          <div class="pick" role="presentation" onmousedown={(e) => e.preventDefault()}>
            {#each tagHits as tag (tag.id)}
              <button type="button" class="pick-item" onclick={() => addLabelName(tag.name)}>
                <span class="dot" style:background={tag.color}></span>
                {tag.name}
              </button>
            {/each}
            {#each profileHits as profile (profile.id)}
              <button type="button" class="pick-item" onclick={() => addBinding(`profile:${profile.id}`)}>
                <ChipMark kind="profile" />
                {profile.name}
              </button>
            {/each}
            {#each workspaceHits as ws (ws.id)}
              <button type="button" class="pick-item" onclick={() => addBinding(`workspace:${ws.id}`)}>
                <span class="dot" style:background={ws.color}></span>
                {ws.name}
              </button>
            {/each}
            {#if canCreate}
              <div class="color-row">
                {#each TAG_COLORS as color (color)}
                  <button
                    type="button"
                    class="swatch"
                    class:active={pickedColor === color}
                    style:background={color}
                    aria-label={color}
                    onclick={() => (pickedColor = color)}
                  ></button>
                {/each}
              </div>
              <button type="button" class="pick-item create" onclick={() => commitQuery()}>
                {$t('notes_tags_add')}
              </button>
            {/if}
          </div>
        {/if}
      </div>

      <!-- Notes link to a TOTP entry only where the notes are (Space); Pass has none. -->
      {#if directory.owned('note')}
        <TotpNoteLinks notes={noteOptions} selected={noteIds} onchange={(ids) => (noteIds = ids)} />
      {/if}

      {#if error && !nameErr && !secretErr}
        <Notice tone="danger">{error}</Notice>
      {/if}
    </div>

  {#snippet footer()}
      <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('common_cancel')}</button>
      <button type="button" class="btn btn-primary" onclick={save} disabled={saving}>
        {saving ? $t('loading') : $t('common_save')}
      </button>
  {/snippet}

</Dialog>

<style>
  /* .tab-bar / .tab styling is global (base.css); only local deltas kept below */
  .tab-bar { padding-top: 0.3rem; margin-bottom: var(--sp-4); }
  .tab { justify-content: center; }

  .totp-form {
    display: flex; flex-direction: column; gap: var(--sp-4);
  }

  .context-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--fs-xs);
    padding: 0.2rem 0.6rem;
    border-radius: var(--radius-sm);
    border-width: 1px;
    border-style: solid;
    font-weight: var(--fw-medium);
  }

  /* Form: .form-group, its label and the fields are the global primitives (base.css), as in the password dialog. */

  /* The «?» note opens under the row of its term; under the row of three selects it spans the row. */
  .label-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .form-row .label-row { position: static; }

  /* The algorithm's option is the longest ("SHA1 (default)"). */
  .form-row { position: relative; display: grid; grid-template-columns: 1.4fr 1fr 1fr; gap: var(--sp-3); }

  .label-chips {
    display: flex; flex-wrap: wrap; gap: 0.35rem; margin-top: 0.3rem;
  }

  .label-chip {
    display: inline-flex; align-items: center; gap: var(--sp-1);
    border-width: 1px; border-style: solid;
    border-radius: var(--radius-pill); font-size: var(--fs-xs); padding: 0.15rem var(--sp-2);
  }

  /* The chip's ink, faded: a fixed --text-2 does not follow the tint. */
  .label-chip button {
    background: none; border: none; cursor: pointer;
    color: inherit; opacity: var(--chip-fade); padding: 0; font-size: var(--fs-base); line-height: var(--lh-icon);
  }
  .label-chip button:hover { opacity: 1; color: var(--danger-text); }

  .context-chip button {
    background: none;
    border: none;
    cursor: pointer;
    color: inherit;
    opacity: var(--chip-fade);
    padding: 0;
    font-size: var(--fs-base);
    line-height: var(--lh-icon);
  }

  .pick {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    margin-top: 0.35rem;
    padding: 0.35rem;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    max-height: 220px;
    overflow-y: auto;
  }

  .pick-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    padding: 0.35rem 0.45rem;
    cursor: pointer;
  }
  .pick-item:hover { background: var(--surface-hover); }
  .pick-item.create { color: var(--accent-text); font-weight: var(--fw-semibold); }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .color-row { display: flex; gap: 0.35rem; padding: 0.25rem 0.45rem; }
  .swatch {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid transparent;
    padding: 0;
    cursor: pointer;
  }
  .swatch.active { border-color: var(--text); }

  /* QR */
  .qr-section {
    display: flex; flex-direction: column; gap: var(--sp-2);
    padding: var(--sp-3); background: var(--surface-3);
    border-radius: var(--radius-md); border: 1px solid var(--border);
  }

  .qr-detected {
    display: flex; align-items: center; gap: 0.4rem;
    font-size: var(--fs-sm); color: var(--success-text); flex-wrap: wrap;
  }

  .masked { color: var(--text-2); font-size: var(--fs-sm); }
  /* The secret is `.mono`; its hint is a sentence in the UI font. */
  #totp-secret::placeholder { font-family: var(--font-ui); }
  .hint { margin: 0; }
</style>
