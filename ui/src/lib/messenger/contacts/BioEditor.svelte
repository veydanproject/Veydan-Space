<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The bio as markup: a text area, a toolbar that puts the marks around the
  selection (bold, italic, strikethrough, code, a colour of the palette),
  a counter of what the runtime counts, and a preview the runtime parses
  as the user types: what it shows is what others will see.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { BIO_COLORS, BIO_MAX_CHARS, messengerApi, type Color, type Span } from '../api';
  import BioView from './BioView.svelte';
  import { applyColor, codePoints, toggleMark, type Mark, type TextSelection } from './profileEdit';

  interface Props {
    value: string;
    disabled?: boolean;
    /** A refusal of the last save about the bio. */
    error?: string;
    id?: string;
  }
  let { value = $bindable(), disabled = false, error = '', id = 'bio' }: Props = $props();

  let area = $state<HTMLTextAreaElement | null>(null);
  let palette = $state(false);
  let preview = $state<Span[]>([]);

  const count = $derived(codePoints(value ?? ''));
  const over = $derived(count > BIO_MAX_CHARS);

  const MARK_BUTTONS: { mark: Mark; icon: string; label: 'msg_profile_bio_bold' | 'msg_profile_bio_italic' | 'msg_profile_bio_strike' | 'msg_profile_bio_code'; keys?: string }[] = [
    { mark: 'bold', icon: 'bold', label: 'msg_profile_bio_bold', keys: 'Ctrl+B' },
    { mark: 'italic', icon: 'italic', label: 'msg_profile_bio_italic', keys: 'Ctrl+I' },
    { mark: 'strike', icon: 'strikethrough', label: 'msg_profile_bio_strike' },
    { mark: 'code', icon: 'code', label: 'msg_profile_bio_code' },
  ];

  // The preview, a moment after the typing stops; a late answer for an
  // older text is dropped.
  $effect(() => {
    const text = value ?? '';
    if (!text.trim()) { preview = []; return; }
    const timer = setTimeout(() => {
      messengerApi.profiles.bioParse(text)
        .then((spans) => { if ((value ?? '') === text) preview = spans; })
        .catch(() => {});
    }, 250);
    return () => clearTimeout(timer);
  });

  async function edit(fn: (s: TextSelection) => TextSelection) {
    if (disabled) return;
    const el = area;
    const s = el ? { value: value ?? '', start: el.selectionStart, end: el.selectionEnd } : { value: value ?? '', start: (value ?? '').length, end: (value ?? '').length };
    const r = fn(s);
    value = r.value;
    await tick();
    if (el) {
      el.focus({ preventScroll: true });
      el.setSelectionRange(r.start, r.end);
    }
  }

  function mark(m: Mark) { void edit((s) => toggleMark(s, m)); }

  function color(c: Color) {
    palette = false;
    void edit((s) => applyColor(s, c));
  }

  function onKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
    const k = e.key.toLowerCase();
    if (k === 'b') { e.preventDefault(); mark('bold'); }
    else if (k === 'i') { e.preventDefault(); mark('italic'); }
  }

  // The tokens by name, so the style guard sees each one.
  const SWATCH: Record<Color, string> = {
    red: 'var(--bio-red)', orange: 'var(--bio-orange)', yellow: 'var(--bio-yellow)', green: 'var(--bio-green)', teal: 'var(--bio-teal)',
    blue: 'var(--bio-blue)', purple: 'var(--bio-purple)', pink: 'var(--bio-pink)', gray: 'var(--bio-gray)',
  };

  // A press on the toolbar keeps the text's selection and the keyboard up.
  const keep = (e: Event) => e.preventDefault();
</script>

<div class="bio-editor" class:invalid={over || !!error}>
  <div class="toolbar" role="toolbar" aria-label={$t('msg_profile_bio')}>
    {#each MARK_BUTTONS as b (b.mark)}
      <button type="button" class="tool" {disabled} onpointerdown={keep} onmousedown={keep} onclick={() => mark(b.mark)}
        title={b.keys ? `${$t(b.label)} (${b.keys})` : $t(b.label)} aria-label={$t(b.label)}>
        <Icon name={b.icon} size={15} />
      </button>
    {/each}
    <span class="sep" aria-hidden="true"></span>
    <button type="button" class="tool" class:active={palette} {disabled} onpointerdown={keep} onmousedown={keep} onclick={() => (palette = !palette)}
      title={$t('msg_profile_bio_color')} aria-label={$t('msg_profile_bio_color')} aria-expanded={palette}>
      <Icon name="palette" size={15} />
    </button>
    <span class="counter" class:over aria-live="polite">{$t('msg_profile_bio_counter', { n: String(count), max: String(BIO_MAX_CHARS) })}</span>
  </div>
  {#if palette}
    <div class="swatches" role="group" aria-label={$t('msg_profile_bio_color')}>
      {#each BIO_COLORS as c (c)}
        <button type="button" class="swatch" style:--sw={SWATCH[c]} onpointerdown={keep} onmousedown={keep} onclick={() => color(c)}
          title={$t(`msg_profile_color_${c}`)} aria-label={$t(`msg_profile_color_${c}`)}></button>
      {/each}
    </div>
  {/if}
  <textarea
    {id}
    bind:this={area}
    bind:value
    rows="5"
    {disabled}
    placeholder={$t('msg_profile_bio_placeholder')}
    aria-invalid={over || !!error}
    onkeydown={onKey}
  ></textarea>
  {#if error}<div class="field-error" role="alert">{error}</div>{:else if over}<div class="field-error">{$t('msg_err_bio_too_long')}</div>{/if}
  <p class="hint">{$t('msg_profile_bio_markup_hint')}</p>
  {#if (value ?? '').trim()}
    <div class="preview">
      <div class="preview-label">{$t('msg_profile_bio_preview')}</div>
      {#if preview.length}<BioView spans={preview} />{:else}<p class="hint">{$t('msg_profile_bio_preview_empty')}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .bio-editor { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .toolbar { display: flex; align-items: center; gap: 2px; flex-wrap: wrap; }
  .tool {
    width: 32px; height: 32px; display: inline-flex; align-items: center; justify-content: center;
    border: 1px solid transparent; border-radius: var(--radius-sm); background: none; color: var(--text-2); cursor: pointer;
  }
  .tool:hover:not(:disabled), .tool.active { background: var(--surface-3); color: var(--text); border-color: var(--border); }
  .tool:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .tool:disabled { opacity: 0.4; cursor: default; }
  .sep { width: 1px; height: 18px; background: var(--border); margin: 0 4px; }
  .counter { margin-left: auto; font-size: var(--fs-xs); color: var(--text-3); font-variant-numeric: tabular-nums; }
  .counter.over { color: var(--danger-text); font-weight: var(--fw-semibold); }
  .swatches {
    display: flex; flex-wrap: wrap; gap: 6px; padding: 8px; border-radius: var(--radius-md);
    background: var(--surface-2); border: 1px solid var(--border);
  }
  .swatch {
    width: 28px; height: 28px; border-radius: 50%; cursor: pointer; padding: 0;
    background: var(--sw); border: 2px solid var(--surface); box-shadow: 0 0 0 1px var(--border-2);
  }
  .swatch:hover { transform: scale(1.08); }
  .swatch:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  textarea { min-height: 6.5em; line-height: 1.45; }
  @media (pointer: fine) { textarea { resize: vertical; } }
  .invalid textarea { border-color: var(--danger-border); }
  .field-error { font-size: var(--fs-xs); color: var(--danger-text); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .preview {
    display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border-radius: var(--radius-md);
    background: var(--surface-2); border: 1px dashed var(--border-2);
  }
  .preview-label { font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.6px; }
  @media (pointer: coarse) {
    .tool { width: 40px; height: 40px; }
    .swatch { width: 36px; height: 36px; }
  }
</style>
