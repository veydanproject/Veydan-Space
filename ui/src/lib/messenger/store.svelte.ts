// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import {
  messengerApi,
  RELAY_STATUS_EVENT,
  RUNTIME_EVENT,
  type IdentityImportKind,
  type MessengerContact,
  type MessengerContactPatch,
  type MessengerIdentity,
  type MessengerLink,
  type MessengerProfile,
  type MessengerProfileInput,
  type MessengerManifestCheck,
  type MessengerManifestInfo,
  type MessengerRelay,
  type MessengerServersMode,
  type MessengerStatus,
  type MessengerUiEvent,
} from './api';
import { chatStore } from './chats/chatStore.svelte';
import { transferStore } from './media/transferStore.svelte';
import { groupStore } from "./groups/groupStore.svelte";
import { nameStore } from "./groups/names.svelte";
import { linkStore } from "./content/linkStore.svelte";
import { sharedStore } from "./content/shared/sharedStore.svelte";
import { netStore } from "./net/netStore.svelte";

export interface FeedEntry extends MessengerUiEvent {
  at: number;
}

const FEED_LIMIT = 50;

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Module-level state: whether the module exists in this build, is enabled, and who we are. */
class MessengerStore {
  status = $state<MessengerStatus | null>(null);
  identity = $state<MessengerIdentity | null>(null);
  relays = $state<MessengerRelay[]>([]);
  manifest = $state<MessengerManifestInfo | null>(null);
  /** Newest first. Runtime events (inbound DMs, ignored events, errors). */
  feed = $state<FeedEntry[]>([]);
  contacts = $state<MessengerContact[]>([]);
  ownProfile = $state<MessengerProfile | null>(null);
  /** One-time encrypted backup of a freshly created key, until the user confirms it is saved. */
  pendingBackup = $state<{ npub: string; ncryptsec: string } | null>(null);
  loaded = $state(false);
  loading = $state(false);
  private _promise: Promise<void> | null = null;
  private _unlisten: (() => void)[] = [];

  /** Show the nav entry only when compiled, enabled and the runtime started. */
  get visible(): boolean {
    const s = this.status;
    return !!s && s.compiled && s.enabled && s.runtime !== null;
  }

  /** Unread messages in chats that are not archived (navigation badge). */
  get unread(): number {
    return this.visible ? chatStore.totalUnread : 0;
  }

  get compiled(): boolean {
    return this.status?.compiled ?? false;
  }

  get secretsUnlocked(): boolean {
    return this.status?.runtime?.secrets_unlocked ?? false;
  }

  get serversMode(): MessengerServersMode | null {
    return this.status?.runtime?.servers_mode ?? null;
  }

  /** Keys first, then whose servers: until both, the module shows the onboarding. */
  get needsOnboarding(): boolean {
    return !!this.status?.runtime && (!this.identity || !!this.pendingBackup || !this.serversMode);
  }

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this.refresh().finally(() => { this._promise = null; });
    return this._promise;
  }

  async refresh() {
    this.loading = true;
    try {
      this.status = await messengerApi.status();
      if (this.visible) {
        const [identity, relays, manifest, contacts, ownProfile] = await Promise.all([
          messengerApi.identity.get(),
          messengerApi.relays.list(),
          messengerApi.relays.manifestInfo(),
          messengerApi.contacts.list().catch(() => [] as MessengerContact[]),
          messengerApi.profiles.ownGet().catch(() => null),
        ]);
        this.identity = identity;
        this.relays = relays;
        this.manifest = manifest;
        this.contacts = contacts;
        this.ownProfile = ownProfile;
        if (identity) {
          chatStore.loadChats().catch(() => {});
          groupStore.load().catch(() => {});
          // Whether a bridge is offered shows above the chats, not only in settings.
          netStore.load().catch(() => {});
          // Events must flow as soon as the module is visible, not only
          // while its page is open (unread badge, statuses).
          this.startListeners().catch(() => {});
        }
      } else {
        this.identity = null;
        this.relays = [];
        this.manifest = null;
        this.contacts = [];
        this.ownProfile = null;
        chatStore.reset();
        groupStore.reset();
        linkStore.reset();
        sharedStore.reset();
        netStore.reset();
      }
      this.loaded = true;
    } finally {
      this.loading = false;
    }
  }

  /**
   * The module was switched off: the runtime is gone, and what the store
   * showed of it goes too. The next start loads again.
   */
  stop() {
    for (const unlisten of this._unlisten) unlisten();
    this._unlisten = [];
    if (this._statusTimer) clearTimeout(this._statusTimer);
    this._statusTimer = null;
    this.status = null;
    this.identity = null;
    this.relays = [];
    this.manifest = null;
    this.contacts = [];
    this.ownProfile = null;
    this.feed = [];
    this.loaded = false;
    chatStore.reset();
    groupStore.reset();
    linkStore.reset();
    sharedStore.reset();
    netStore.reset();
  }

  /** Subscribe to relay-state and runtime event pushes. Idempotent. */
  async startListeners() {
    if (this._unlisten.length || !isTauri) return;
    const { listen } = await import('@tauri-apps/api/event');
    this._unlisten.push(
      await listen<MessengerRelay[]>(RELAY_STATUS_EVENT, (e) => {
        this.relays = e.payload;
        this.scheduleStatusRefresh();
      }),
      await listen<MessengerUiEvent>(RUNTIME_EVENT, (e) => {
        chatStore.handleEvent(e.payload);
        transferStore.handleEvent(e.payload);
        groupStore.handleEvent(e.payload);
        linkStore.handleEvent(e.payload);
        sharedStore.handleEvent(e.payload);
        netStore.handleEvent(e.payload);
        if (e.payload.name === 'transfer.progress') return;
        this.feed = [{ ...e.payload, at: Date.now() }, ...this.feed].slice(0, FEED_LIMIT);
        if (e.payload.name === 'dm.message' || e.payload.name === 'history.synced') this.scheduleStatusRefresh();
        if (e.payload.name === 'link') {
          // The dot changes at once; the rest of the status follows.
          const link = (e.payload.payload as { state?: MessengerLink } | null)?.state;
          if (link && this.status?.runtime) this.status = { ...this.status, runtime: { ...this.status.runtime, link } };
          this.scheduleStatusRefresh();
        }
        if (e.payload.name === "profile.updated") {
          const pk = (e.payload.payload as { pubkey?: string } | null)?.pubkey;
          if (pk) nameStore.refresh(pk);
        }
        if (e.payload.name === 'profile.updated' || e.payload.name === 'follows.updated') {
          this.refreshContacts().catch(() => {});
        }
      }),
    );
  }

  private _statusTimer: ReturnType<typeof setTimeout> | null = null;

  /** Status for the header dot and diagnostics; bursts collapse into one call. */
  scheduleStatusRefresh() {
    if (this._statusTimer) return;
    this._statusTimer = setTimeout(() => {
      this._statusTimer = null;
      messengerApi.status().then((s) => (this.status = s)).catch(() => {});
    }, 1000);
  }

  async refreshContacts() {
    const [contacts, ownProfile] = await Promise.all([messengerApi.contacts.list(), messengerApi.profiles.ownGet()]);
    this.contacts = contacts;
    this.ownProfile = ownProfile;
  }

  async addContact(key: string, nickname?: string) {
    const c = await messengerApi.contacts.add(key, nickname);
    await this.refreshContacts();
    return c;
  }

  async updateContact(pubkey: string, patch: MessengerContactPatch) {
    await messengerApi.contacts.update(pubkey, patch);
    await this.refreshContacts();
  }

  async removeContact(pubkey: string) {
    await messengerApi.contacts.remove(pubkey);
    await this.refreshContacts();
  }

  async setFollowed(pubkey: string, followed: boolean) {
    await messengerApi.contacts.setFollowed(pubkey, followed);
    await this.refreshContacts();
  }

  async requestProfile(pubkey: string) {
    await messengerApi.profiles.request(pubkey);
  }

  async verifyNip05(pubkey: string) {
    const ok = await messengerApi.profiles.verifyNip05(pubkey);
    await this.refreshContacts();
    return ok;
  }

  async saveOwnProfile(input: MessengerProfileInput) {
    this.ownProfile = await messengerApi.profiles.ownSet(input);
    return this.ownProfile;
  }

  clearFeed() {
    this.feed = [];
  }

  async sendTextDm(to: string, text: string) {
    return (await messengerApi.dm.sendText(to, text)).id;
  }

  async refreshRelays() {
    const [relays, manifest] = await Promise.all([messengerApi.relays.list(), messengerApi.relays.manifestInfo()]);
    this.relays = relays;
    this.manifest = manifest;
  }

  async addRelay(url: string, apiKey?: string) {
    await messengerApi.relays.add(url, apiKey);
    await this.refreshRelays();
  }

  async removeRelay(url: string) {
    await messengerApi.relays.remove(url);
    await this.refreshRelays();
  }

  async setRelayEnabled(url: string, enabled: boolean) {
    await messengerApi.relays.setEnabled(url, enabled);
    await this.refreshRelays();
  }

  async setSilent(enabled: boolean) {
    await messengerApi.relays.setSilent(enabled);
    await this.refresh();
  }

  async setRegion(region: string) {
    await messengerApi.relays.setRegion(region);
    await this.refresh();
  }

  /** The project's servers: its signed manifest, or the built-in one. */
  async useVeydanServers(): Promise<MessengerManifestCheck> {
    const check = await messengerApi.relays.useVeydan();
    await this.refresh();
    return check;
  }

  /** Only the user's servers; the project is never asked for anything. */
  async useOwnServers() {
    await messengerApi.relays.useOwn();
    await this.refresh();
  }

  async refreshManifest(): Promise<MessengerManifestCheck | null> {
    const check = await messengerApi.relays.refreshManifest();
    await this.refresh();
    return check;
  }

  async createIdentity(password: string) {
    const created = await messengerApi.identity.create(password);
    this.pendingBackup = { npub: created.identity.npub, ncryptsec: created.ncryptsec };
    this.identity = created.identity;
    await this.refresh();
    return created;
  }

  ackBackup() {
    this.pendingBackup = null;
  }

  async importIdentity(kind: IdentityImportKind, secret: string, password?: string) {
    this.identity = await messengerApi.identity.import(kind, secret, password);
    await this.refresh();
    return this.identity;
  }

  exportIdentity(password: string) {
    return messengerApi.identity.export(password);
  }

  async deleteIdentity() {
    await messengerApi.identity.delete();
    this.identity = null;
    await this.refresh();
  }
}

export const messengerStore = new MessengerStore();
