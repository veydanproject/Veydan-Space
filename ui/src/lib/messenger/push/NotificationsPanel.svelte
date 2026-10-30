<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { date } from '../shared/time';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerTranslations } from '../i18n';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import type { MessengerPushTicket } from '../api';
  import { hostOf } from '../servers/serverAccept.svelte';
  import { serverStore } from '../servers/serverStore.svelte';
  import { pushErrorKey, pushStore } from './pushStore.svelte';

  let editingServer = $state(false);
  let serverDraft = $state('');

  onMount(() => {
    void (async () => {
      await pushStore.load();
      pushStore.loadNotify();
      // A computer: no push, the app shows notifications itself.
      if (pushStore.view && !pushStore.view.device.supported) pushStore.loadDesk();
    })();
    // Back from the phone's settings: whether messages ring is read again.
    const back = () => {
      if (document.visibilityState === 'visible' && pushStore.view?.device.supported) void pushStore.load();
    };
    document.addEventListener('visibilitychange', back);
    return () => document.removeEventListener('visibilitychange', back);
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

  /** The push of my private servers, through the gateway; their tickets are read once there is one. */
  const targets = $derived(status?.enabled ? (status.targets ?? []) : []);
  let ticketsAsked = false;
  $effect(() => {
    if (targets.length && !ticketsAsked) {
      ticketsAsked = true;
      void pushStore.loadTickets();
    }
  });

  async function revoke(ticket: MessengerPushTicket) {
    const yes = await ask({
      title: $t('msg_push_ticket_revoke_title', { ticket: ticket.ticket }),
      message: $t('msg_push_ticket_revoke_text'),
      confirmLabel: $t('msg_push_ticket_revoke'),
    });
    if (yes) await pushStore.revokeTicket(ticket.ticket);
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
      <div class="seg choices" role="radiogroup" aria-label={$t('msg_notify_title')}>
        {#each ['sender_text', 'sender', 'none'] as const as choice (choice)}
          <button
            class="seg-btn"
            class:active={n.content === choice}
            role="radio"
            aria-checked={n.content === choice}
            disabled={pushStore.busy || n.locked}
            onclick={() => pushStore.setNotify(choice, n.lockscreen_hidden)}
          >{$t(`msg_notify_${choice}` as 'msg_notify_sender_text')}</button>
        {/each}
      </div>
      {#if phone || n.lockscreen_supported}
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
              <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.keepRunning()}>
                {$t('msg_dnotify_keep')}
              </button>
            </div>
          </div>
        {/if}
        <div class="actions">
          <button
            class="btn btn-ghost"
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

    <!-- Notifications switched off, or a channel of messages turned quiet in
         the phone's settings: told whether pushes are on or not, since the
         app shows messages itself too and the toggle may be absent. -->
    {#if device.quiet}
      <div class="line">
        <span class="warn">{$t('msg_push_quiet')}</span>
        <button class="btn btn-ghost btn-sm" disabled={pushStore.busy} onclick={() => pushStore.openSettings()}>
          {$t('msg_push_quiet_fix')}
        </button>
      </div>
    {/if}

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

      {#if !status.enabled && device.permission === 'denied' && !device.quiet}
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
            <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.sendTest()}>
              {$t('msg_push_test')}
            </button>
            <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.refresh()}>
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
            <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => pushStore.refresh()}>
              {$t('msg_push_refresh')}
            </button>
          </div>
        {/if}

        {#if targets.length}
          <div class="block">
            <span class="label">{$t('msg_push_targets_title')}</span>
            <p class="muted small">{$t('msg_push_targets_hint')}</p>
            <ul class="list">
              {#each targets as target (target.server_id)}
                <li class="row">
                  <span class="dot {target.state === 'registered' ? 'ok' : target.state === 'pending' ? 'pending' : 'failed'}"></span>
                  <div class="info">
                    <span class="name">{target.name?.trim() || serverStore.nameOf(target.server_id) || hostOf(target.server)}</span>
                    <span class="meta">
                      {#if target.state === 'registered'}
                        {$t('msg_push_target_registered', { gateway: target.gateway ? hostOf(target.gateway) : '—' })}
                      {:else}
                        {tr(`msg_push_target_${target.state === 'pending' ? 'pending' : 'failed'}`)}
                      {/if}
                    </span>
                    {#if target.state === 'failed' && target.error}<span class="meta bad">{errorText(target.error)}</span>{/if}
                    {#if target.ticket_expires_at}<span class="meta">{$t('msg_push_target_ticket', { date: day(target.ticket_expires_at) })}</span>{/if}
                  </div>
                </li>
              {/each}
            </ul>
          </div>
        {/if}

        {#if targets.length || pushStore.tickets?.length}
          <div class="block">
            <span class="label">{$t('msg_push_tickets_title')}</span>
            <p class="muted small">{$t('msg_push_tickets_hint')}</p>
            {#if pushStore.tickets === null}
              <p class="muted small">{$t('msg_push_loading')}</p>
            {:else if pushStore.tickets.length === 0}
              <p class="muted small">{$t('msg_push_tickets_empty')}</p>
            {:else}
              <ul class="list">
                {#each pushStore.tickets as ticket (ticket.gateway + ticket.ticket)}
                  <li class="row">
                    <div class="info">
                      <span class="name">{ticket.server_id ? serverStore.nameOf(ticket.server_id) || ticket.server_id : $t('msg_push_ticket_other')}</span>
                      <code>{ticket.ticket} · {hostOf(ticket.gateway)}</code>
                      <span class="meta">
                        {$t('msg_push_ticket_meta', { until: day(ticket.expires_at), woke: ticket.last_wake_at ? day(ticket.last_wake_at) : $t('msg_push_ticket_never') })}
                      </span>
                    </div>
                    <button class="btn btn-ghost danger" disabled={pushStore.busy} onclick={() => revoke(ticket)}>
                      {$t('msg_push_ticket_revoke')}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
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
            class="btn btn-primary"
            disabled={pushStore.busy || !serverDraft.trim()}
            onclick={() => saveServer(serverDraft.trim())}
          >
            {$t('msg_push_server_save')}
          </button>
          {#if status.server_custom}
            <button class="btn btn-ghost" disabled={pushStore.busy} onclick={() => saveServer(null)}>
              {$t('msg_push_server_reset')}
            </button>
          {/if}
          <button class="btn btn-ghost" onclick={() => (editingServer = false)}>{$t('msg_back')}</button>
        </div>
      {:else}
        <div class="line">
          <div class="info">
            <code>{status.server ?? '—'}</code>
            <span class="meta">
              {status.server_custom ? $t('msg_push_server_custom') : $t('msg_push_server_default')}
            </span>
          </div>
          <button class="btn btn-ghost" onclick={startEditing}>{$t('msg_push_server_change')}</button>
        </div>
      {/if}
    </div>

    {#if pushStore.error}<p class="warn">{errorText(pushStore.error)}</p>{/if}
  {/if}
</div>

<style>
  .push { display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); min-height: 40px; font-size: var(--fs-base); }
  .line > .btn { flex-shrink: 0; }
  .state { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .state.ok { color: var(--success-text); }
  .state.bad, .warn { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  /* Each part of the card under a line (as «Профиль и ключ»). */
  .block { display: flex; flex-direction: column; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  /* Three choices wrap their words on a phone instead of running off the row. */
  .choices { align-self: flex-start; max-width: 100%; }
  .choices .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); }
  /* Rows of 48 with a line between them (as the facts of «Профиль и ключ»). */
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; padding: var(--sp-2) 0; }
  .row + .row { border-top: 1px solid var(--border); }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  .meta.bad { color: var(--danger-text); }
  .name { font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .btn.danger { color: var(--danger-text); flex-shrink: 0; }
  .dot.failed { background: var(--danger-text); }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body); word-break: break-all; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex-shrink: 0; background: var(--danger-text); }
  .dot.ok { background: var(--success-text); }
  .dot.pending { background: var(--color-warning); }
  .dot.unknown { background: var(--text-3); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  input {
    min-height: 44px; font: inherit; font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: 0 var(--sp-3);
  }
  input:focus { outline: none; border-color: var(--accent-border); }
</style>
