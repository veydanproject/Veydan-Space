<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { date } from '../shared/time';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerTranslations } from '../i18n';
  import { pushErrorKey, pushStore } from './pushStore.svelte';

  let editingServer = $state(false);
  let serverDraft = $state('');

  onMount(async () => {
    await pushStore.load();
    pushStore.loadNotify();
    // A computer: no push, the app shows notifications itself.
    if (pushStore.view && !pushStore.view.device.supported) pushStore.loadDesk();
  });

  const view = $derived(pushStore.view);
  const status = $derived(view?.status ?? null);
  const device = $derived(view?.device ?? null);
  const desktop = $derived(Boolean(device && !device.supported));

  const reasonText: Record<string, string> = {
    no_firebase_config: 'msg_push_reason_no_config',
    no_play_services: 'msg_push_reason_no_play',
    token_failed: 'msg_push_reason_token_failed',
  };

  const known = (key: string) => key in messengerTranslations.en;
  // A key built from what the server said: checked against the dictionary
  // before it is used.
  const tr = (key: string, vars?: Record<string, string>) => $t(key as 'msg_push_title', vars);

  function errorText(error: string): string {
    return tr(pushErrorKey(error, known), { error });
  }

  function day(secs: number | null): string {
    return secs ? date(secs) : '—';
  }

  function startEditing() {
    serverDraft = status?.server_custom ? (status.server ?? '') : '';
    editingServer = true;
  }

  async function saveServer(url: string | null) {
    await pushStore.setServer(url);
    if (!pushStore.error) editingServer = false;
  }
</script>

{#snippet contentChoices(phone: boolean)}
  {#if pushStore.notify}
    {@const n = pushStore.notify}
    <div class="block">
      <span class="label">{$t('msg_notify_title')}</span>
      {#if n.locked}
        <p class="muted small">{$t('msg_notify_locked')}</p>
      {:else}
        <p class="muted small">{$t(phone ? 'msg_notify_text' : 'msg_dnotify_content_text')}</p>
      {/if}
      <div class="choices" role="radiogroup" aria-label={$t('msg_notify_title')}>
        {#each ['sender_text', 'sender', 'none'] as const as choice (choice)}
          <button
            class="choice"
            class:on={n.content === choice}
            role="radio"
            aria-checked={n.content === choice}
            disabled={pushStore.busy || n.locked}
            onclick={() => pushStore.setNotify(choice, n.lockscreen_hidden)}
          >{$t(`msg_notify_${choice}` as 'msg_notify_sender_text')}</button>
        {/each}
      </div>
      {#if phone}
        <div class="line">
          <span>{$t('msg_notify_lockscreen')}</span>
          <button
            class="toggle"
            class:on={n.lockscreen_hidden}
            disabled={pushStore.busy || n.locked}
            onclick={() => pushStore.setNotify(n.content, !n.lockscreen_hidden)}
            aria-pressed={n.lockscreen_hidden}
            aria-label={$t('msg_notify_lockscreen')}
          ></button>
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

<div class="card push">
  <div class="card-title"><Icon name="bell" size={16} /> {desktop ? $t('msg_dnotify_title') : $t('msg_push_title')}</div>

  {#if !view || !status || !device}
    <p class="muted">{pushStore.error || $t('msg_push_loading')}</p>
  {:else if !device.supported}
    {@const d = pushStore.desk}
    <p class="muted">{$t('msg_dnotify_text')}</p>
    {#if !d}
      <p class="muted">{pushStore.error || $t('msg_push_loading')}</p>
    {:else}
      <div class="line">
        <span>{$t('msg_dnotify_toggle')}</span>
        <button
          class="toggle"
          class:on={d.enabled}
          disabled={pushStore.busy}
          onclick={() => pushStore.setDesk(!d.enabled, d.sound)}
          aria-pressed={d.enabled}
          aria-label={$t('msg_dnotify_toggle')}
        ></button>
      </div>
      {#if d.enabled}
        {#if !d.available}<p class="warn">{$t('msg_dnotify_unavailable')}</p>{/if}
        <div class="line">
          <span>{$t('msg_dnotify_sound')}</span>
          <button
            class="toggle"
            class:on={d.sound}
            disabled={pushStore.busy}
            onclick={() => pushStore.setDesk(d.enabled, !d.sound)}
            aria-pressed={d.sound}
            aria-label={$t('msg_dnotify_sound')}
          ></button>
        </div>
        {@render contentChoices(false)}
        {#if !d.close_to_tray}
          <div class="block">
            <p class="warn">{$t('msg_dnotify_closing')}</p>
            <div class="actions">
              <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.keepRunning()}>
                {$t('msg_dnotify_keep')}
              </button>
            </div>
          </div>
        {/if}
        <div class="actions">
          <button
            class="btn btn-ghost btn-sm"
            disabled={pushStore.busy || !d.available}
            onclick={() => pushStore.testDesk($t('app_name'), $t('msg_dnotify_test_body'))}
          >
            {$t('msg_dnotify_test')}
          </button>
        </div>
      {/if}
    {/if}
    {#if pushStore.error}<p class="warn">{errorText(pushStore.error)}</p>{/if}
  {:else}
    <p class="muted">{$t('msg_push_text')}</p>
    <p class="muted small">{$t('msg_push_privacy')}</p>

    {#if !device.available}
      <p class="warn">
        {tr(reasonText[device.reason ?? ''] ?? 'msg_push_reason_unknown')}
        {#if device.detail}<span class="small"> ({device.detail})</span>{/if}
      </p>
    {:else}
      <div class="line">
        <span>{$t('msg_push_toggle')}</span>
        <button
          class="toggle"
          class:on={status.enabled}
          disabled={pushStore.busy}
          onclick={() => pushStore.setEnabled(!status.enabled)}
          aria-pressed={status.enabled}
          aria-label={$t('msg_push_toggle')}
        ></button>
      </div>

      {#if !status.enabled && device.permission === 'denied'}
        <p class="warn">{$t('msg_push_refused')}</p>
      {/if}

      {#if status.enabled}
        <p class="state" class:ok={status.state === 'registered'} class:bad={status.state === 'failed'}>
          {tr(`msg_push_state_${status.state}`)}
        </p>
        {#if status.error}<p class="warn">{errorText(status.error)}</p>{/if}
        {#if status.last_ok_at}
          <p class="muted small">
            {$t('msg_push_told', { date: day(status.last_ok_at), until: day(status.expires_at) })}
          </p>
        {/if}

        <div class="line">
          <span>{$t('msg_push_pref_dm')}</span>
          <button
            class="toggle"
            class:on={status.dm}
            disabled={pushStore.busy}
            onclick={() => pushStore.setPrefs(!status.dm, status.groups)}
            aria-pressed={status.dm}
            aria-label={$t('msg_push_pref_dm')}
          ></button>
        </div>
        <div class="line">
          <span>{$t('msg_push_pref_groups')}</span>
          <button
            class="toggle"
            class:on={status.groups}
            disabled={pushStore.busy}
            onclick={() => pushStore.setPrefs(status.dm, !status.groups)}
            aria-pressed={status.groups}
            aria-label={$t('msg_push_pref_groups')}
          ></button>
        </div>

        {@render contentChoices(true)}

        {#if status.state === 'registered'}
          <div class="block">
            <span class="label">{$t('msg_push_relays_title')}</span>
            {#if status.relays.every((r) => r.status !== 'ok' && r.status !== 'pending')}
              <p class="warn">{$t('msg_push_relays_none')}</p>
            {/if}
            <ul class="list">
              {#each status.relays as r (r.url)}
                <li class="row">
                  <span class="dot {r.status}"></span>
                  <div class="info">
                    <code>{r.url}</code>
                    <span class="meta">{tr(`msg_push_relay_${r.status}`)}</span>
                  </div>
                </li>
              {/each}
            </ul>
          </div>

          <div class="actions">
            <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.sendTest()}>
              {$t('msg_push_test')}
            </button>
            <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.refresh()}>
              {$t('msg_push_refresh')}
            </button>
          </div>
          {#if pushStore.test}
            <p class="muted small">
              {tr(`msg_push_test_${pushStore.test.outcome}`, { trace: pushStore.test.trace })}
            </p>
          {/if}
        {:else if status.state === 'failed'}
          <div class="actions">
            <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.refresh()}>
              {$t('msg_push_refresh')}
            </button>
          </div>
        {/if}
      {/if}
    {/if}

    <div class="block">
      <span class="label">{$t('msg_push_server')}</span>
      {#if editingServer}
        <p class="muted small">{$t('msg_push_server_hint')}</p>
        <input
          type="url"
          inputmode="url"
          autocapitalize="off"
          autocomplete="off"
          spellcheck="false"
          placeholder="https://"
          bind:value={serverDraft}
        />
        <div class="actions">
          <button
            class="btn btn-primary btn-sm"
            disabled={pushStore.busy || !serverDraft.trim()}
            onclick={() => saveServer(serverDraft.trim())}
          >
            {$t('msg_push_server_save')}
          </button>
          {#if status.server_custom}
            <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => saveServer(null)}>
              {$t('msg_push_server_reset')}
            </button>
          {/if}
          <button class="btn btn-ghost btn-sm" onclick={() => (editingServer = false)}>{$t('msg_back')}</button>
        </div>
      {:else}
        <div class="line">
          <div class="info">
            <code>{status.server ?? '—'}</code>
            <span class="meta">
              {status.server_custom ? $t('msg_push_server_custom') : $t('msg_push_server_default')}
            </span>
          </div>
          <button class="btn btn-ghost btn-sm" onclick={startEditing}>{$t('msg_push_server_change')}</button>
        </div>
      {/if}
    </div>

    {#if pushStore.error}<p class="warn">{errorText(pushStore.error)}</p>{/if}
  {/if}
</div>

<style>
  .push { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); }
  .state { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .state.ok { color: var(--success-text); }
  .state.bad, .warn { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .choices { display: flex; gap: var(--sp-1); flex-wrap: wrap; }
  .choice {
    font: inherit; font-size: var(--fs-xs); color: var(--text-2); cursor: pointer;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-pill); padding: 4px 10px;
  }
  .choice.on { background: var(--accent-tint); color: var(--accent-text-2); border-color: transparent; }
  .choice:disabled { opacity: 0.5; cursor: default; }
  .label { font-size: var(--fs-xs); color: var(--text-3); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row {
    display: flex; align-items: center; gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--surface);
  }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body); word-break: break-all; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex-shrink: 0; background: var(--danger-text); }
  .dot.ok { background: var(--success-text); }
  .dot.pending { background: var(--color-warning); }
  .dot.unknown { background: var(--text-3); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  input {
    font: inherit; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  input:focus { outline: none; border-color: var(--accent-border); }
</style>
