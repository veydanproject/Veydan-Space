<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My avatar in the profile editor: pick a picture (the system's picker,
  images only, so a phone shows its photo picker; on a computer a file
  dropped on it too), crop it, and the runtime re-encodes, uploads and
  publishes it. There is no address to type: only the runtime sets one.
  The avatar takes effect at once, apart from the form's Publish.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { isTauriHost, messengerApi, profileLabel, type AvatarPreview, type CropRect } from '../api';
  import { messengerStore } from '../store.svelte';
  import Avatar from './Avatar.svelte';
  import AvatarCropper from './AvatarCropper.svelte';
  import { profileCodeOf, profileErrorText } from './profileErrors';

  interface Props {
    disabled?: boolean;
    /** Shows the media servers' settings (an avatar needs one to be stored on). */
    onopenmedia?: () => void;
  }
  let { disabled = false, onopenmedia }: Props = $props();

  const EXTENSIONS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp'];
  const IMAGE_FILE = /\.(jpe?g|png|webp|gif|bmp)$/i;

  const p = $derived(messengerStore.ownProfile);
  const label = $derived(profileLabel(p) || (messengerStore.identity?.npub ?? '?'));

  let phase = $state<'idle' | 'preparing' | 'cropping' | 'uploading' | 'removing'>('idle');
  let picked = $state<AvatarPreview | null>(null);
  let rect = $state<CropRect | undefined>();
  let cropOpen = $state(false);
  /** A refusal of the picker or of the remove: under the avatar. */
  let error = $state('');
  /** A refusal of the upload: inside the crop dialog. */
  let cropError = $state('');
  let noServer = $state(false);
  let dropOver = $state(false);
  let zone = $state<HTMLElement | null>(null);

  const busy = $derived(phase === 'preparing' || phase === 'uploading' || phase === 'removing');

  async function pick() {
    if (disabled || busy) return;
    error = '';
    let source: string | null;
    if (!isTauriHost) {
      // The browser preview has no paths: a sample the demo draws.
      source = '/home/dev/Pictures/avatar-sample.jpg';
    } else {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const res = await open({
        multiple: false, directory: false, title: $t('msg_profile_avatar_pick_title'),
        filters: [{ name: $t('msg_profile_avatar_filter'), extensions: EXTENSIONS }],
      }).catch(() => null);
      source = typeof res === 'string' ? res : null;
    }
    if (source) await prepare(source);
  }

  async function prepare(source: string) {
    phase = 'preparing'; error = ''; cropError = ''; noServer = false;
    try {
      picked = await messengerApi.avatar.prepare(source);
      phase = 'cropping';
      cropOpen = true;
    } catch (e) {
      phase = 'idle';
      error = profileErrorText(e);
    }
  }

  async function apply() {
    if (!picked || !rect || phase === 'uploading') return;
    phase = 'uploading'; cropError = ''; noServer = false;
    try {
      await messengerStore.setAvatar(picked.token, rect);
      cropOpen = false;
      picked = null;
      phase = 'idle';
    } catch (e) {
      if (!cropOpen) {
        // The dialog was closed while it uploaded: the refusal goes under the avatar.
        picked = null; phase = 'idle'; error = profileErrorText(e);
        return;
      }
      phase = 'cropping';
      noServer = profileCodeOf(e) === 'media_no_server';
      cropError = profileErrorText(e);
    }
  }

  function cancel() {
    if (phase === 'uploading') return;
    cropOpen = false;
    picked = null;
    cropError = ''; noServer = false;
    phase = 'idle';
  }

  function openMedia() {
    cancel();
    onopenmedia?.();
  }

  async function remove() {
    if (disabled || busy) return;
    const yes = await ask({ title: $t('msg_profile_avatar_remove'), message: $t('msg_profile_avatar_remove_confirm'), confirmLabel: $t('msg_profile_avatar_remove') });
    if (!yes) return;
    phase = 'removing'; error = '';
    try { await messengerStore.removeAvatar(); }
    catch (e) { error = profileErrorText(e); }
    finally { phase = 'idle'; }
  }

  // A file dropped on the avatar (a computer). Tauri gives its path and
  // lets the runtime read it; nothing else (a picture dragged from a web
  // page) is taken.
  const dropScale = () => (navigator.userAgent.includes('Linux') ? 1 : window.devicePixelRatio || 1);
  function inZone(x: number, y: number): boolean {
    const r = zone?.getBoundingClientRect();
    if (!r) return false;
    const s = dropScale();
    const px = x / s, py = y / s;
    // A margin around it: the target is small.
    return px >= r.left - 24 && px <= r.right + 24 && py >= r.top - 24 && py <= r.bottom + 24;
  }

  // A file dropped from the desktop; a touch screen has nothing to drop.
  const canDrop = isTauriHost && !(typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches);

  onMount(() => {
    if (!canDrop) return;
    let un: (() => void) | null = null;
    let disposed = false;
    void import('@tauri-apps/api/webview').then(({ getCurrentWebview }) =>
      getCurrentWebview().onDragDropEvent((event) => {
        const ev = event.payload;
        if (ev.type === 'leave') { dropOver = false; return; }
        if (ev.type === 'enter' || ev.type === 'over') {
          dropOver = !disabled && !busy && inZone(ev.position.x, ev.position.y);
          return;
        }
        if (ev.type === 'drop') {
          const over = inZone(ev.position.x, ev.position.y);
          dropOver = false;
          if (!over || disabled || busy) return;
          const file = ev.paths.length === 1 ? ev.paths[0] : null;
          if (file && IMAGE_FILE.test(file)) void prepare(file);
          else error = $t('msg_err_avatar_unsupported');
        }
      }),
    ).then((u) => { if (disposed) u(); else un = u; }).catch(() => {});
    return () => { disposed = true; un?.(); };
  });
</script>

<div class="avatar-pick">
  <div class="zone" class:over={dropOver} bind:this={zone}>
    <button type="button" class="face" disabled={disabled || busy} onclick={pick} title={p?.picture ? $t('msg_profile_avatar_change') : $t('msg_profile_avatar_upload')} aria-label={p?.picture ? $t('msg_profile_avatar_change') : $t('msg_profile_avatar_upload')}>
      <Avatar url={p?.picture ?? null} {label} seed={messengerStore.identity?.pubkey} size={88} />
      <span class="badge-cam" aria-hidden="true">
        {#if busy}<span class="spinner"></span>{:else}<Icon name="camera" size={16} />{/if}
      </span>
    </button>
    {#if dropOver}<div class="drop">{$t('msg_profile_avatar_drop')}</div>{/if}
  </div>
  <div class="side">
    <div class="buttons">
      <button type="button" class="btn btn-ghost btn-sm" disabled={disabled || busy} onclick={pick}>
        <Icon name="upload" size={13} />{p?.picture ? $t('msg_profile_avatar_change') : $t('msg_profile_avatar_upload')}
      </button>
      {#if p?.picture}
        <button type="button" class="btn btn-ghost btn-sm danger" disabled={disabled || busy} onclick={remove}>
          <Icon name="trash" size={13} />{$t('msg_profile_avatar_remove')}
        </button>
      {/if}
    </div>
    {#if phase === 'preparing'}
      <p class="hint">{$t('msg_profile_avatar_preparing')}</p>
    {:else}
      <p class="hint">{$t('msg_profile_avatar_hint')}{canDrop ? ` ${$t('msg_profile_avatar_paste_hint')}` : ''}</p>
    {/if}
    {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
  </div>
</div>

<Dialog bind:open={cropOpen} title={$t('msg_profile_crop_title')} closeOnBackdrop={false} onclose={cancel} width="min(400px, calc(100vw - 24px))">
  {#if picked}
    <div class="crop-body">
      <AvatarCropper src={picked.preview} width={picked.width} height={picked.height} disabled={phase === 'uploading'} bind:rect />
      <p class="hint center">{$t('msg_profile_avatar_public_hint')}</p>
      {#if phase === 'uploading'}
        <p class="status"><span class="spinner"></span>{$t('msg_profile_avatar_uploading')}</p>
      {/if}
      {#if cropError}
        <div class="error-msg" role="alert">
          {noServer ? $t('msg_profile_no_media_server') : cropError}
          {#if noServer && onopenmedia}
            <button type="button" class="btn btn-ghost btn-sm media-btn" onclick={openMedia}>
              <Icon name="settings" size={13} />{$t('msg_profile_open_media_settings')}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" disabled={phase === 'uploading'} onclick={cancel}>{$t('msg_profile_crop_cancel')}</button>
    <button type="button" class="btn btn-primary" disabled={phase !== 'cropping' || !rect} onclick={apply}>
      {#if phase === 'uploading'}<span class="spinner"></span>{:else}<Icon name="check" size={14} />{/if}{$t('msg_profile_crop_apply')}
    </button>
  {/snippet}
</Dialog>

<style>
  .avatar-pick { display: flex; gap: var(--sp-4); align-items: center; }
  .zone { position: relative; flex-shrink: 0; border-radius: 50%; }
  .zone.over { box-shadow: 0 0 0 3px var(--accent); }
  .face {
    position: relative; display: block; padding: 0; border: none; background: none; border-radius: 50%; cursor: pointer;
  }
  .face:disabled { cursor: default; }
  .face:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
  .face:hover:not(:disabled) :global(.avatar) { filter: brightness(0.92); }
  .badge-cam {
    position: absolute; right: -2px; bottom: -2px; width: 32px; height: 32px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center;
    background: var(--accent-grad); color: #fff; border: 2px solid var(--surface); box-shadow: var(--shadow);
  }
  .badge-cam .spinner { border-color: color-mix(in srgb, #fff 35%, transparent); border-top-color: #fff; }
  .drop {
    position: absolute; inset: 0; border-radius: 50%; display: flex; align-items: center; justify-content: center; text-align: center;
    padding: 6px; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); line-height: 1.2;
    background: color-mix(in srgb, var(--accent) 78%, transparent); color: #fff; pointer-events: none;
  }
  .side { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .buttons { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .btn :global(svg) { margin-right: 2px; }
  .danger:hover:not(:disabled) { color: var(--danger-text); border-color: var(--danger-border); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .center { text-align: center; }
  .crop-body { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); }
  .crop-body .error-msg { align-self: stretch; display: flex; flex-direction: column; gap: var(--sp-2); align-items: flex-start; }
  .status { margin: 0; display: inline-flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-2); }
  .media-btn { color: var(--text); }
  @media (max-width: 420px) {
    .avatar-pick { flex-direction: column; text-align: center; }
    .buttons { justify-content: center; }
  }
</style>
