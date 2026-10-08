// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import {
  messengerApi,
  onDemoEvent,
  RELAY_STATUS_EVENT,
  RUNTIME_EVENT,
  type IdentityImportKind,
  type MessengerContact,
  type MessengerContactPatch,
  type MessengerIdentity,
  type MessengerLink,
  type MessengerProfile,
  type MessengerProfileInput,
  type CropRect,
  type MessengerManifestCheck,
  type MessengerManifestInfo,
  type MessengerRelay,
  type MessengerServersMode,
  type MessengerStatus,
  type MessengerUiEvent,
  type OwnPrivateView,
} from './api';
import { avatarStore } from './contacts/avatars.svelte';
import { chatStore } from './chats/chatStore.svelte';
import { transferStore } from './media/transferStore.svelte';
import { groupStore } from "./groups/groupStore.svelte";
import { nameStore } from "./groups/names.svelte";
import { linkStore } from "./content/linkStore.svelte";
import { sharedStore } from "./content/shared/sharedStore.svelte";
import { netStore } from "./net/netStore.svelte";
import { usageStore } from "./shared/emoji/usageStore.svelte";
import { presenceStore } from "./presence/presenceStore.svelte";
import { privacyStore } from "./privacy/privacyStore.svelte";
import { pushSeen } from "./push/bridge";
import { callStore } from "./calls/callStore.svelte";
import { groupCallStore } from "./calls/groupCallStore.svelte";

export interface FeedEntry extends MessengerUiEvent {
  at: number;
}

const FEED_LIMIT = 50;
/** Half the runtime's 90 s lease on "the page is seen". */
const PRESENCE_LEASE_MS = 45_000;

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
  /** My phone and whether my card carries it; `null` until `loadOwnPrivate`. Never published. */
  ownPrivate = $state<OwnPrivateView | null>(null);
  /** Phones contacts sent me in their own cards, by hex key, as far as asked (`contactPhone`). */
  contactPhones = $state<Record<string, string | null>>({});
  private phonesAsked = new Set<string>();
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
          // Popular emoji come from every device of mine: reactions and the picker start from them.
          usageStore.load().catch(() => {});
          // Who of my contacts is online; the clock of "last seen" starts with it.
          presenceStore.load().catch(() => {});
          presenceStore.start();
          // A call under way (the phone's Answer may have started the app for it), the policy, the nodes.
          callStore.load().catch(() => {});
          // The room of a group call I am in (a page that came back finds it).
          groupCallStore.load().catch(() => {});
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
        this.ownPrivate = null;
        this.contactPhones = {};
        this.phonesAsked.clear();
        avatarStore.reset();
        chatStore.reset();
        groupStore.reset();
        linkStore.reset();
        sharedStore.reset();
        netStore.reset();
        usageStore.reset();
        presenceStore.reset();
        transferStore.reset();
        callStore.reset();
        groupCallStore.reset();
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
    this.ownPrivate = null;
    this.contactPhones = {};
    this.phonesAsked.clear();
    this.feed = [];
    avatarStore.reset();
    this.loaded = false;
    chatStore.reset();
    groupStore.reset();
    linkStore.reset();
    sharedStore.reset();
    netStore.reset();
    usageStore.reset();
    presenceStore.reset();
    transferStore.reset();
    callStore.reset();
    groupCallStore.reset();
  }

  /** Subscribe to relay-state and runtime event pushes. Idempotent. */
  async startListeners() {
    if (this._unlisten.length) return;
    if (!isTauri) {
      // The browser preview plays its file transfers (api.ts).
      this._unlisten.push(onDemoEvent((ev) => {
        chatStore.handleEvent(ev);
        transferStore.handleEvent(ev);
        callStore.handleEvent(ev);
        groupCallStore.handleEvent(ev);
      }));
      return;
    }
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
        usageStore.handleEvent(e.payload);
        presenceStore.handleEvent(e.payload);
        privacyStore.handleEvent(e.payload);
        avatarStore.handleEvent(e.payload);
        callStore.handleEvent(e.payload);
        groupCallStore.handleEvent(e.payload);
        this.handleProfileEvent(e.payload);
        // Many a second while a file moves or a peer speaks: not for the feed.
        if (e.payload.name === 'transfer.progress' || e.payload.name === 'call.level' || e.payload.name === 'group_call.level') return;
        this.feed = [{ ...e.payload, at: Date.now() }, ...this.feed].slice(0, FEED_LIMIT);
        if (e.payload.name === 'dm.message' || e.payload.name === 'history.synced') this.scheduleStatusRefresh();
        if (e.payload.name === 'link') {
          // The dot changes at once; the rest of the status follows.
          const link = (e.payload.payload as { state?: MessengerLink } | null)?.state;
          if (link && this.status?.runtime) this.status = { ...this.status, runtime: { ...this.status.runtime, link } };
          this.scheduleStatusRefresh();
        }
        if (e.payload.name === 'chat.read') {
          // Read on another device of mine: the phone's notification of it goes too.
          const chatId = (e.payload.payload as { chat_id?: string } | null)?.chat_id;
          if (chatId) pushSeen(chatId);
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
    this._unlisten.push(this.leaseForeground());
  }

  /**
   * The runtime beats presence only while the page is seen. `true` is a
   * lease of 90 s, said again every 45 s, so a page that died without a
   * word stops the heartbeat by itself; hiding says `false` at once.
   * Returns what undoes it.
   */
  private leaseForeground(): () => void {
    const visible = () => document.visibilityState === 'visible';
    const say = (v: boolean) => messengerApi.presence.foreground(v).catch(() => {});
    const onChange = () => say(visible());
    document.addEventListener('visibilitychange', onChange);
    say(visible());
    const timer = setInterval(() => { if (visible()) say(true); }, PRESENCE_LEASE_MS);
    return () => {
      document.removeEventListener('visibilitychange', onChange);
      clearInterval(timer);
    };
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

  /** Cropped picture (token of `messengerApi.avatar.prepare`) → my avatar, published. */
  async setAvatar(token: string, rect: CropRect) {
    this.ownProfile = await messengerApi.avatar.set(token, rect);
    return this.ownProfile;
  }

  async removeAvatar() {
    this.ownProfile = await messengerApi.avatar.remove();
    return this.ownProfile;
  }

  async loadOwnPrivate() {
    this.ownPrivate = await messengerApi.ownPrivate.get();
    return this.ownPrivate;
  }

  /** Error: `phone_invalid`. */
  async saveOwnPrivate(phone: string | null, sharePhone: boolean) {
    this.ownPrivate = await messengerApi.ownPrivate.set(phone, sharePhone);
    return this.ownPrivate;
  }

  /**
   * The phone `pubkey` sent me in its own card: `undefined` until known,
   * then kept fresh by `contact_private.updated`. Reading it asks once.
   */
  contactPhone(pubkey: string): string | null | undefined {
    if (!this.phonesAsked.has(pubkey)) {
      this.phonesAsked.add(pubkey);
      queueMicrotask(() => this.loadContactPhone(pubkey).catch(() => {}));
    }
    return this.contactPhones[pubkey];
  }

  async loadContactPhone(pubkey: string) {
    const v = await messengerApi.contactPrivate.get(pubkey);
    this.contactPhones[v.pubkey] = v.phone;
    if (v.pubkey !== pubkey) this.contactPhones[pubkey] = v.phone;
    return v.phone;
  }

  /** Private parts changed on another device of mine, or a card was accepted. */
  private handleProfileEvent(ev: MessengerUiEvent) {
    if (ev.name === 'own_private.updated' && this.ownPrivate) this.loadOwnPrivate().catch(() => {});
    if (ev.name === 'contact_private.updated') {
      const pk = (ev.payload as { pubkey?: string } | null)?.pubkey;
      if (pk && pk in this.contactPhones) this.loadContactPhone(pk).catch(() => {});
    }
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
