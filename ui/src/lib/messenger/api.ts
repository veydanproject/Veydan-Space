// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Messenger command layer. Intentionally separate from `$lib/api` so the
// module can be lifted into a standalone app: this file is the only place
// that knows command names (docs/messenger-spec.md §4.6).

// Types the runtime writes for itself (`make msg-types`); never by hand.
export type { GroupMembership, LinkGroupKind, LinkPreview, LinkView } from './generated/links';
export type { SharedCounts, SharedSection } from './generated/shared';
export type { BridgeView, NetCheck, NetMode, NetStatus, Verdict } from './generated/net';
import type { NetCheck, NetMode, NetStatus } from './generated/net';

export interface MessengerIngressCounters {
  received: number;
  duplicates: number;
  dispatched: number;
  dm: number;
  ignored: number;
}

export interface MessengerRuntimeStatus {
  version: string;
  data_dir: string;
  schema_version: number;
  secrets_unlocked: boolean;
  identity_present: boolean;
  session_active: boolean;
  relays_total: number;
  relays_connected: number;
  silent_mode: boolean;
  manifest_serial: number | null;
  region: string;
  /** Whose servers are used; `null` until the user chooses (onboarding). */
  servers_mode: MessengerServersMode | null;
  ingress: MessengerIngressCounters;
  outbox_pending: number;
}

export interface MessengerProfile {
  pubkey: string;
  npub: string;
  name: string | null;
  display_name: string | null;
  about: string | null;
  picture: string | null;
  banner: string | null;
  website: string | null;
  nip05: string | null;
  lud16: string | null;
  nip05_verified: boolean;
  event_created_at: number;
  fetched_at: number;
}

export interface MessengerProfileInput {
  name?: string | null;
  display_name?: string | null;
  about?: string | null;
  picture?: string | null;
  banner?: string | null;
  website?: string | null;
  nip05?: string | null;
  lud16?: string | null;
}

export interface MessengerContact {
  pubkey: string;
  npub: string;
  nickname: string | null;
  note: string | null;
  followed: boolean;
  profile: MessengerProfile | null;
  created_at: number;
  updated_at: number;
}

export interface MessengerContactPatch {
  nickname?: string | null;
  note?: string | null;
}

/** Display label: nickname → profile name → short npub. */
export function contactLabel(c: MessengerContact): string {
  return c.nickname?.trim() || profileLabel(c.profile) || `${c.npub.slice(0, 12)}…${c.npub.slice(-4)}`;
}

export function profileLabel(p: MessengerProfile | null): string {
  if (!p) return '';
  return p.display_name?.trim() || p.name?.trim() || p.nip05 || '';
}


export type ChatMode =
  | 'full_chat' | 'first_contact' | 'request_sent' | 'request_received' | 'request_declined'
  | 'request_declined_by_me' | 'request_revoked_by_peer' | 'removed_by_peer' | 'both_removed'
  | 'mutual_reconnect' | 'blocked' | 'blocked_by_peer'
  /** Not a relationship: the chat is a group. */
  | 'group';

export interface MessengerChat {
  id: string;
  kind: 'dm' | 'group' | string;
  peer_pubkey: string | null;
  peer_npub: string | null;
  title: string;
  picture: string | null;
  is_contact: boolean;
  is_muted: boolean;
  unread: number;
  last_message_at: number | null;
  last_preview: string | null;
  pinned: boolean;
  archived: boolean;
  mode: ChatMode;
  can_send: boolean;
}

export interface MessengerReplyPreview {
  id: string;
  sender_pubkey: string;
  text: string | null;
}

export type MessageStatus = 'queued' | 'sent' | 'failed' | 'received' | 'uploading' | 'paused';

export interface MessengerMessage {
  id: string;
  chat_id: string;
  direction: 'in' | 'out';
  status: MessageStatus;
  content_type: string;
  text: string | null;
  sender_pubkey: string;
  reply_to: MessengerReplyPreview | null;
  created_at: number;
  edited_at: number | null;
  deleted: boolean;
  failure_reason: string | null;
  media: Record<string, unknown> | null;
}



export type GroupRole = 'owner' | 'admin' | 'moderator' | 'member';

export interface MessengerGroupMember {
  pubkey: string;
  role: GroupRole;
  muted: boolean;
  joined_at: number;
  is_me: boolean;
}

export interface MessengerGroup {
  id: string;
  chat_id: string;
  kind: 'public' | 'private';
  name: string;
  about: string;
  picture: string;
  relay: string;
  owner: string;
  membership: GroupMembership;
  my_role: GroupRole | null;
  muted: boolean;
  can_post: boolean;
  history_for_new: boolean;
  members: MessengerGroupMember[];
  /** Managers only. */
  banned: string[];
  /** Managers only: who asks to be let in. */
  requests: string[];
  /** For those who may share it. */
  link: string | null;
  /** Events this device has no key for (yet). */
  undecrypted: number;
  /** The key messages are sealed with now (members only). */
  key: MessengerGroupKey | null;
}

/** What a member may know about the current group key: never the key itself. */
export interface MessengerGroupKey {
  /** Short fingerprint of the key. */
  id: string;
  /** 1 for the key the group was created with, then one more per change. */
  version: number;
  cipher: string;
  source: 'random' | 'link';
  link_epoch: number;
  /** When it came, unix seconds by its author's clock. */
  since: number;
  by: string;
  reason: 'create' | 'rotate_key' | 'rotate_link' | 'remove' | 'ban' | 'admit' | 'other';
  /** This device holds it. */
  held: boolean;
  status: 'good' | 'rotate' | 'deliver';
}

export interface MessengerGroupInvite {
  invite_id: string;
  group_id: string;
  name: string;
  about: string;
  picture: string;
  members: number;
  peer: string;
  direction: 'in' | 'out';
  status: string;
  created_at: number;
  expires_at: number;
}


/** Stable refusal code of a preview (`preview_silent`, …), if any. */
export function previewErrorCode(e: unknown): string | null {
  const m = /\bpreview_[a-z_]+\b/.exec(messengerError(e));
  return m ? m[0] : null;
}

/** An operation on a group, as its log writes it. */
export type GroupAction =
  | { op: 'remove' | 'ban' | 'unban'; who: string }
  | { op: 'set_role'; who: string; role: GroupRole }
  | { op: 'set_muted'; who: string; muted: boolean }
  | { op: 'edit_settings'; name?: string; about?: string; picture?: string; history_for_new?: boolean }
  | { op: 'transfer_ownership'; to: string }
  | { op: 'leave' | 'disband' };

const ROLE_RANK: Record<GroupRole, number> = { member: 0, moderator: 1, admin: 2, owner: 3 };
export const roleRank = (r: GroupRole | null | undefined): number => (r ? ROLE_RANK[r] : -1);

/** Stable refusal code inside an error (`group_not_permitted`, …), if any. */
export function groupErrorCode(e: unknown): string | null {
  const m = /\bgroup_[a-z_]+\b/.exec(messengerError(e));
  return m ? m[0] : null;
}

export type DmAction = 'request' | 'accept' | 'decline' | 'block' | 'unblock' | 'remove';

export interface MessengerRelation {
  peer_pubkey: string;
  mode: ChatMode;
  my_contact: 'none' | 'approved' | 'declined';
  blocked: boolean;
  peer_signal: 'none' | 'approved' | 'blocked' | 'left' | 'declined' | 'revoked';
  was_ever_mutual: boolean;
  can_send: boolean;
}

/** Stable refusal code inside an error (`dm_waiting_approval`, …), if any. */
export function dmErrorCode(e: unknown): string | null {
  const m = /\bdm_[a-z_]+\b/.exec(messengerError(e));
  return m ? m[0] : null;
}


export type MediaKind = 'image' | 'video' | 'audio' | 'file' | 'voice' | 'circle';

/** Attachment fields of a message as the UI needs them. */
export interface MessengerMedia {
  name: string;
  mime: string;
  size: number;
  kind: MediaKind;
  /** Present when the file is on this device. */
  local_path?: string;
  transfer_id?: string;
  /** Recordings: length and loudness outline (0..255 per bar). */
  duration_ms?: number;
  waveform?: number[];
}

export function mediaOf(m: MessengerMessage): MessengerMedia | null {
  const f = m.media;
  if (!f || typeof f.name !== 'string') return null;
  return {
    name: f.name,
    mime: typeof f.mime === 'string' ? f.mime : 'application/octet-stream',
    size: typeof f.size === 'number' ? f.size : 0,
    kind: (["image", "video", "audio", "file", "voice", "circle"] as const).includes(f.kind as MediaKind) ? (f.kind as MediaKind) : "file",
    local_path: typeof f.local_path === 'string' ? f.local_path : undefined,
    transfer_id: typeof f.transfer_id === 'string' ? f.transfer_id : undefined,
    duration_ms: typeof f.duration_ms === "number" ? f.duration_ms : undefined,
    waveform: Array.isArray(f.waveform) ? (f.waveform as unknown[]).filter((x): x is number => typeof x === "number") : undefined,
  };
}

export type TransferStatus = 'queued' | 'running' | 'paused' | 'done' | 'failed' | 'cancelled';

export interface MessengerTransfer {
  id: string;
  direction: 'up' | 'down';
  message_id: string | null;
  chat_id: string | null;
  file_name: string;
  mime: string;
  size: number;
  status: TransferStatus;
  done_bytes: number;
  attempts: number;
  failure_reason: string | null;
  local_path: string | null;
}

/** Payload of the `transfer.progress` runtime event. */
export interface MessengerTransferProgress {
  transfer_id: string;
  message_id: string | null;
  chat_id: string | null;
  direction: 'up' | 'down';
  status: TransferStatus;
  done_bytes: number;
  total_bytes: number;
  failure_reason: string | null;
  local_path: string | null;
}

export interface MessengerMediaServer {
  id: string;
  kind: 's3' | 'blossom';
  url: string;
  bucket: string | null;
  region: string | null;
  access_key: string | null;
  has_secret: boolean;
  priority: number;
  enabled: boolean;
  source: 'manifest' | 'user';
  public_base: string;
}

/** A voice message or a video circle as the recorder produced it. */
export interface MessengerRecording {
  kind: "voice" | "circle";
  mime: string;
  duration_ms: number;
  waveform?: number[];
  blob: Blob;
}

async function blobToBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = "";
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) bin += String.fromCharCode(...bytes.subarray(i, i + step));
  return btoa(bin);
}

export interface MessengerMediaServerInput {
  id?: string | null;
  kind: 's3' | 'blossom';
  url: string;
  bucket?: string | null;
  region?: string | null;
  access_key?: string | null;
  secret_key?: string | null;
}

/** Stable failure code inside an error (`err.rate_limited`, …), if any. */
export function mediaErrorCode(e: unknown): string | null {
  const m = /\berr\.[a-z_]+\b/.exec(typeof e === 'string' ? e : messengerError(e));
  return m ? m[0] : null;
}

/** Runtime UI event as forwarded by the host. */
export interface MessengerUiEvent {
  name: string;
  payload: unknown;
}

/** Event name: payload is `MessengerUiEvent`. */
export const RUNTIME_EVENT = 'messenger://event';

export type RelayState = 'disconnected' | 'connecting' | 'connected' | 'paused';

export interface MessengerRelay {
  url: string;
  relay_id: string | null;
  source: 'manifest' | 'user';
  regions: string[];
  read: boolean;
  write: boolean;
  enabled: boolean;
  /** 'nip42' | 'api_key' | null. The key itself never reaches the UI. */
  auth_type: string | null;
  state: RelayState;
}

export interface MessengerManifestInfo {
  serial: number | null;
  issued_at: number | null;
  region: string;
  regions: string[];
  silent_mode: boolean;
  mode: MessengerServersMode | null;
  /** Where the manifest in use came from: its URL, or `embedded`. */
  origin: string;
  /** When the project was last asked for a newer one (unix seconds). */
  checked_at: number | null;
}

export type MessengerServersMode = 'veydan' | 'own';

/** What asking the project for its manifest found. */
export interface MessengerManifestCheck {
  origin: string;
  serial: number;
  updated: boolean;
  /** Why the project's manifest could not be fetched (the built-in one is used). */
  error: string | null;
}

/** Event name: payload is `MessengerRelay[]`. */
export const RELAY_STATUS_EVENT = 'messenger://relay-status';

export type PushPermission = 'granted' | 'denied' | 'prompt' | 'prompt-with-rationale';

/** What the phone says about pushes. */
export interface MessengerPushDevice {
  /** False on a desktop: there is no push service to talk to. */
  supported: boolean;
  /** False when this phone cannot receive pushes; `reason` says why. */
  available: boolean;
  /** 'no_firebase_config' | 'no_play_services' | 'token_failed' */
  reason: string | null;
  detail: string | null;
  permission: PushPermission | null;
}

/** `unknown` is a word of a newer server. */
export type PushRelayStatus =
  | 'ok' | 'pending' | 'not_allowed' | 'invalid' | 'restricted' | 'unreachable' | 'unknown';

/** What the push server does with a relay of the user. */
export interface MessengerPushRelay {
  url: string;
  status: PushRelayStatus;
  detail?: string | null;
}

export type PushStateName =
  | 'off' | 'paused' | 'waiting_unlock' | 'no_channel' | 'no_server'
  | 'pending' | 'registered' | 'failed';

/** Where pushes stand with the push server. */
export interface MessengerPushStatus {
  /** The user agreed to pushes. */
  enabled: boolean;
  /** The user was asked, whatever the answer. */
  offered: boolean;
  server: string | null;
  /** The server was named by the user, not by the manifest. */
  server_custom: boolean;
  dm: boolean;
  groups: boolean;
  state: PushStateName;
  /** Unix seconds. */
  last_ok_at: number | null;
  expires_at: number | null;
  /** `push_unreachable: …`, `push_refused_<code>: … (request <id>)` */
  error: string | null;
  relays: MessengerPushRelay[];
}

export interface MessengerPushView {
  device: MessengerPushDevice;
  status: MessengerPushStatus;
}

/** What the push service answered to a test push. */
export interface MessengerPushTest {
  /** 'delivered' | 'dead_token' | 'rejected' | 'retry' */
  outcome: string;
  /** Id of the push in the server's log. */
  trace: string;
}

/** The server's word in `push_refused_<code>`, or the kind of failure. */
export function pushErrorCode(e: unknown): string | null {
  const m = /\bpush_[a-z_]+\b/.exec(messengerError(e));
  return m ? m[0] : null;
}

/** A notification the user tapped. */
/** What a notification may say; kept by the messenger, read by the push handler. */
export interface MessengerNotifySettings {
  content: 'sender_text' | 'sender' | 'none';
  lockscreen_hidden: boolean;
  /** A PIN or password guards the app: notifications say only that something came, whatever `content` says. */
  locked: boolean;
}

/** Notifications of this computer: the app shows them itself while it runs. */
export interface MessengerDesktopNotify {
  enabled: boolean;
  sound: boolean;
  /** The system can show notifications (a notification server, an app bundle). */
  available: boolean;
  /** Closing the window keeps the app running; otherwise nothing comes after it. */
  close_to_tray: boolean;
}

/**
 * The words a notification of the system is made of, `{n}` left for the
 * number. A notification without a title of its own is titled with the
 * product's name, which the backend knows (platform-spec 13.3).
 */
export interface MessengerNoticeWords {
  new_message: string;
  new_messages: string;
  more: string;
  request: string;
  group_invite: string;
  group_request: string;
  group_welcome: string;
  photo: string;
  video: string;
  voice: string;
  circle: string;
  audio: string;
  file: string;
  /** `{n}` is how many. */
  album: string;
  /** `{n}` is how many. */
  files: string;
  /** `{name}` is the group's. */
  link_group: string;
  link_group_nameless: string;
  /** `{name}` is the person's. */
  link_contact: string;
  link_contact_nameless: string;
}

export interface MessengerPushTap {
  type: string;
  /** `dm:<pubkey>` or `group:<id>`; null when the phone could not say whose the message was. */
  chat: string | null;
}

/** Event name, no payload: ask `push.takeTap()`. */
export const PUSH_TAP_EVENT = 'messenger://push-tap';

export interface MessengerStatus {
  /** False when the host binary was built without the `messenger` feature. */
  compiled: boolean;
  /** Host setting: the module is shown in navigation. */
  enabled: boolean;
  runtime: MessengerRuntimeStatus | null;
  error: string | null;
}

export interface MessengerIdentity {
  npub: string;
  pubkey: string;
  created_at: number;
}

export interface MessengerCreatedIdentity {
  identity: MessengerIdentity;
  /** NIP-49 backup string; shown once, never stored by the UI. */
  ncryptsec: string;
}

export type IdentityImportKind = 'nsec' | 'ncryptsec' | 'mnemonic';

/** Error shape forwarded by the host (`{ code, message }`). */
export interface MessengerError {
  code: string;
  message?: string;
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const NOT_COMPILED: MessengerStatus = { compiled: false, enabled: false, runtime: null, error: null };

import { buildDemo, demoEnabled } from './devDemo';
import type { ExternalUrl, InternalLinkText } from './content/types';
import type { GroupMembership, LinkPreview, LinkView } from './generated/links';
import type { SharedCounts, SharedSection } from './generated/shared';
import { inSection } from './content/shared/sections';

const demo = !isTauri && demoEnabled() ? buildDemo() : null;
let mockIdentity: MessengerIdentity | null = demo?.identity ?? null;
function manifestRelay(): MessengerRelay {
  return { url: 'wss://node-1.veydan.net', relay_id: 'veydan-node-1', source: 'manifest', regions: ['default', 'ru'], read: true, write: true, enabled: true, auth_type: 'api_key', state: 'connected' };
}
let mockRelays: MessengerRelay[] = [manifestRelay()];
let mockNotify: MessengerNotifySettings = { content: 'sender_text', lockscreen_hidden: false, locked: false };
let mockDesktopNotify: MessengerDesktopNotify = { enabled: true, sound: true, available: true, close_to_tray: false };
// `messenger.demo.desk=1`: the demo is a computer (no push, notifications of its own).
const mockDesk = typeof localStorage !== 'undefined' && localStorage.getItem('messenger.demo.desk') === '1';
let mockPush: MessengerPushView = {
  device: { supported: !mockDesk, available: !mockDesk, reason: null, detail: null, permission: 'prompt' },
  status: {
    enabled: false, offered: false, server: 'https://vpush.veydan.net', server_custom: false,
    dm: true, groups: true, state: 'off', last_ok_at: null, expires_at: null, error: null, relays: [],
  },
};
let mockSilent = false;
let mockRegion = 'default';
// `messenger.demo.net=restricted`: the direct way does not carry, a bridge does.
const mockRestricted = typeof localStorage !== 'undefined' && localStorage.getItem('messenger.demo.net') === 'restricted';
let mockNet: NetStatus = {
  mode: 'off', active: false, bridge: null, bridges: 2, private: [], list_checked_at: null, offer: mockRestricted, available: true,
};
function mockNetApply(): NetStatus {
  const active = mockNet.mode === 'on' || (mockNet.mode === 'auto' && mockRestricted);
  mockNet = { ...mockNet, active, bridge: active ? '45.93.201.244:443' : null, bridges: 2 + mockNet.private.length, available: mockServersMode !== 'own' };
  return mockNet;
}
function mockBridgeOf(text: string): { id: string; addr: string } | null {
  const link = /^veydan:\/\/vlink\/([0-9a-fA-F]{64})\?a=([^&]+)$/.exec(text.trim());
  if (link) return { id: link[1].toLowerCase(), addr: decodeURIComponent(link[2]) };
  const ref = /^([0-9.]+:[0-9]+)#([0-9a-fA-F]{64})$/.exec(text.trim());
  return ref ? { id: ref[2].toLowerCase(), addr: ref[1] } : null;
}
// A fresh install chooses its servers in the onboarding; the demo already has.
let mockServersMode: MessengerServersMode | null = demo ? 'veydan' : null;
let mockManifestOrigin = 'embedded';
let mockCheckedAt: number | null = null;
// `messenger.demo.manifest=ok`: the project's signed manifest "answers".
const mockManifestOk = typeof localStorage !== 'undefined' && localStorage.getItem('messenger.demo.manifest') === 'ok';
function mockManifestCheck(): MessengerManifestCheck {
  mockCheckedAt = Math.floor(Date.now() / 1000);
  if (mockManifestOk) mockManifestOrigin = 'https://veydan.net/messenger/manifest.json';
  return { origin: mockManifestOrigin, serial: mockManifestOk ? 5 : 4, updated: mockManifestOk, error: mockManifestOk ? null : 'https://veydan.net/messenger/manifest.json: http 404' };
}
let mockContacts: MessengerContact[] = demo?.contacts ?? [];
let mockOwnProfile: MessengerProfile | null = demo?.ownProfile ?? null;
const emptyProfile = (pubkey: string): MessengerProfile => ({
  pubkey, npub: `npub1${pubkey.slice(0, 58)}`, name: null, display_name: null, about: null, picture: null, banner: null,
  website: null, nip05: null, lud16: null, nip05_verified: false, event_created_at: 0, fetched_at: 0,
});

let mockChats: MessengerChat[] = demo?.chats ?? [];
let mockMediaServers: MessengerMediaServer[] = [
  { id: 'veydan-node-1-media', kind: 'blossom', url: 'https://node-1.veydan.net/media', bucket: null, region: null, access_key: null, has_secret: false, priority: 10, enabled: true, source: 'manifest', public_base: 'https://node-1.veydan.net/media' },
];
const mockMessages: Record<string, MessengerMessage[]> = demo?.messages ?? {};
let mockGroups: MessengerGroup[] = demo?.groups ?? [];
let mockInvites: MessengerGroupInvite[] = demo?.invites ?? [];
function mockGroup(id: string): MessengerGroup {
  const g = mockGroups.find((x) => x.id === id);
  if (!g) throw new Error("group_unknown");
  return g;
}
function mockChat(peer: string): MessengerChat {
  if (peer.startsWith('group:')) {
    const g = mockChats.find((x) => x.id === peer);
    if (g) return g;
  }
  const hex = peer.startsWith('npub') ? 'ef'.repeat(32) : peer;
  const id = `dm:${hex}`;
  let c = mockChats.find((x) => x.id === id);
  if (!c) {
    const contact = mockContacts.find((x) => x.pubkey === hex);
    c = { id, kind: 'dm', peer_pubkey: hex, peer_npub: `npub1${hex.slice(0, 58)}`, title: contact ? contactLabel(contact) : `npub1${hex.slice(0, 7)}…`, picture: null, is_contact: !!contact, is_muted: false, unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false, mode: 'full_chat', can_send: true };
    mockChats = [c, ...mockChats];
    mockMessages[id] = [];
  }
  return c;
}
function mockFind(id: string): MessengerMessage | undefined {
  return Object.values(mockMessages).flat().find((m) => m.id === id);
}

/** Browser preview only: in the app links are taken apart by the runtime. */
function mockInspect(text: string): LinkView {
  const link = text.trim();
  const npub = /^(?:nostr:)?(npub1[0-9a-z]+)$/.exec(link)?.[1] ?? /^veydan:\/\/contact\/(npub1[0-9a-z]+)(?:\?|$)/.exec(link)?.[1];
  if (npub) {
    const c = mockContacts.find((x) => x.npub === npub);
    const me = mockIdentity?.npub === npub;
    const hint = new URLSearchParams(link.split('?')[1] ?? '').get('n') ?? '';
    return {
      kind: 'contact', link: link.startsWith('veydan://') ? link : `veydan://contact/${npub}`, pubkey: c?.pubkey ?? mockIdentity?.pubkey ?? 'ef'.repeat(32), npub,
      name: c ? contactLabel(c) : me ? profileLabel(mockOwnProfile) : hint, picture: null, nip05: c?.profile?.nip05 ?? null,
      is_me: me, is_contact: !!c, blocked: false,
    };
  }
  const m = /^veydan:\/\/([a-z]+)\/([A-Za-z0-9._~-]+)(?:\?(.*))?$/.exec(link);
  if (!m) return { kind: 'invalid', code: link.startsWith('veydan://') ? 'link_bad_id' : 'link_bad_scheme' };
  if (m[1] === 'vlink') {
    const bridge = mockBridgeOf(link);
    return bridge ? { kind: 'vlink', link, id: bridge.id, addr: bridge.addr, added: mockNet.private.some((b) => b.id === bridge.id) } : { kind: 'invalid', code: 'link_bad_param' };
  }
  if (m[1] !== 'group') return m[1] === 'contact' ? { kind: 'invalid', code: 'link_bad_id' } : { kind: 'unknown', link_type: m[1] };
  const p = new URLSearchParams(m[3] ?? '');
  const t = p.get('t');
  if (!/^[0-9a-f]{64}$/.test(m[2]) || (t !== 'public' && t !== 'private') || !p.get('r') || !p.get('o')) return { kind: 'invalid', code: 'link_bad_param' };
  const g = mockGroups.find((x) => x.id === m[2]);
  return {
    kind: 'group', link, group_id: m[2], group_kind: t, name: g?.name || p.get('n') || '', relay: p.get('r') ?? '', owner: p.get('o') ?? '',
    picture: g?.picture || null, members: g?.members.length || null, membership: g?.membership ?? null,
  };
}

const devMocks: Record<string, (args?: Record<string, unknown>) => unknown> = {
  messenger_status: () => ({
    compiled: true,
    enabled: true,
    runtime: {
      version: '0.1.0-dev',
      data_dir: '/home/dev/.local/share/net.veydan.space/data/messenger',
      schema_version: 2,
      secrets_unlocked: true,
      identity_present: mockIdentity !== null,
      session_active: mockIdentity !== null,
      relays_total: mockRelays.filter((r) => r.enabled).length,
      relays_connected: mockRelays.filter((r) => r.state === 'connected').length,
      silent_mode: mockSilent,
      manifest_serial: mockServersMode === 'veydan' ? (mockManifestOk ? 5 : 4) : null,
      region: mockRegion,
      servers_mode: mockServersMode,
      ingress: { received: 0, duplicates: 0, dispatched: 0, dm: 0, ignored: 0 },
      outbox_pending: 0,
    },
    error: null,
  }),
  messenger_chats_list: () => mockChats,
  messenger_chat_open: (a) => mockChat(String(a?.peer)),
  messenger_chat_messages: (a) => {
    const all = mockMessages[String(a?.chatId)] ?? [];
    const before = (a?.before as number | null) ?? Number.MAX_SAFE_INTEGER;
    return all.filter((m) => m.created_at < before).slice(-((a?.limit as number) ?? 50));
  },
  messenger_chat_shared_counts: (a): SharedCounts => {
    const all = mockMessages[String(a?.chatId)] ?? [];
    const n = (s: SharedSection) => all.filter((m) => inSection(m, s)).length;
    return { visual: n('visual'), files: n('files'), links: n('links'), voice: n('voice') };
  },
  messenger_chat_shared: (a) => {
    const all = mockMessages[String(a?.chatId)] ?? [];
    const before = (a?.before as number | null) ?? Number.MAX_SAFE_INTEGER;
    return all.filter((m) => m.created_at < before && inSection(m, a?.section as SharedSection))
      .reverse().slice(0, (a?.limit as number) ?? 60);
  },
  messenger_chat_mark_read: (a) => { mockChats = mockChats.map((c) => c.id === a?.chatId ? { ...c, unread: 0 } : c); },
  messenger_chat_set_pinned: (a) => { mockChats = mockChats.map((c) => c.id === a?.chatId ? { ...c, pinned: Boolean(a?.pinned) } : c); },
  messenger_chat_set_archived: (a) => { mockChats = mockChats.map((c) => c.id === a?.chatId ? { ...c, archived: Boolean(a?.archived) } : c); },
  messenger_chat_set_muted: (a) => { mockChats = mockChats.map((c) => c.id === a?.chatId ? { ...c, is_muted: Boolean(a?.muted) } : c); },
  messenger_chat_delete: (a) => { mockChats = mockChats.filter((c) => c.id !== a?.chatId); },
  messenger_dm_send_text: (a) => {
    const c = mockChat(String(a?.to));
    const now = Math.floor(Date.now() / 1000);
    const target = a?.replyTo ? mockFind(String(a.replyTo)) : undefined;
    const m: MessengerMessage = { id: `${Date.now().toString(16)}${Math.random().toString(16).slice(2)}`, chat_id: c.id, direction: 'out', status: 'sent', content_type: 'text', text: String(a?.text), sender_pubkey: 'ab'.repeat(32), reply_to: target ? { id: target.id, sender_pubkey: target.sender_pubkey, text: target.text } : null, created_at: now, edited_at: null, deleted: false, failure_reason: null, media: null };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    mockChats = mockChats.map((x) => x.id === c.id ? { ...x, last_message_at: now, last_preview: m.text } : x);
    return m;
  },
  messenger_dm_edit: (a) => { const m = mockFind(String(a?.messageId)); if (m) { m.text = String(a?.text); m.edited_at = Math.floor(Date.now() / 1000); } return m; },
  messenger_dm_delete: (a) => { const m = mockFind(String(a?.messageId)); if (m) { m.deleted = true; m.text = null; } },
  messenger_dm_retry: () => undefined,
  messenger_media_servers: () => mockMediaServers,
  messenger_media_server_put: (a) => {
    const i = (a?.input ?? {}) as MessengerMediaServerInput;
    const id = i.id ?? `${i.kind}-${mockMediaServers.length + 1}`;
    const s: MessengerMediaServer = { id, kind: i.kind, url: i.url, bucket: i.bucket ?? null, region: i.region ?? (i.kind === 's3' ? 'us-east-1' : null), access_key: i.access_key ?? null, has_secret: !!i.secret_key, priority: 10, enabled: true, source: 'user', public_base: i.kind === 's3' ? `${i.url}/${i.bucket}` : i.url };
    mockMediaServers = [...mockMediaServers.filter((x) => x.id !== id), s];
    return s;
  },
  messenger_media_server_remove: (a) => { mockMediaServers = mockMediaServers.filter((x) => x.id !== a?.id); },
  messenger_media_server_set_enabled: (a) => { mockMediaServers = mockMediaServers.map((x) => (x.id === a?.id ? { ...x, enabled: Boolean(a?.enabled) } : x)); },
  messenger_media_server_check: () => undefined,
  messenger_dm_send_file: (a) => {
    const c = mockChat(String(a?.to));
    const now = Math.floor(Date.now() / 1000);
    const name = String(a?.path).split(/[\\/]/).pop() ?? 'file';
    const ext = name.split('.').pop()?.toLowerCase() ?? '';
    const kind = ['jpg', 'jpeg', 'png', 'gif', 'webp'].includes(ext) ? 'image' : ['mp4', 'webm'].includes(ext) ? 'video' : ['mp3', 'ogg', 'wav'].includes(ext) ? 'audio' : 'file';
    const m: MessengerMessage = { id: `local:${Date.now().toString(16)}${Math.random().toString(16).slice(2, 8)}`, chat_id: c.id, direction: 'out', status: 'sent', content_type: 'media', text: (a?.caption as string) ?? null, sender_pubkey: 'ab'.repeat(32), reply_to: null, created_at: now, edited_at: null, deleted: false, failure_reason: null, media: { name, mime: 'application/octet-stream', size: 1_234_567, kind, local_path: String(a?.path), ...(a?.batch ? { batch: String(a.batch) } : {}) } };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    mockChats = mockChats.map((x) => (x.id === c.id ? { ...x, last_message_at: now, last_preview: `📎 ${name}` } : x));
    return m;
  },
  messenger_media_grant_access: () => undefined,
  messenger_dm_send_recording: (a) => {
    const c = mockChat(String(a?.to));
    const r = (a?.recording ?? {}) as { kind: MediaKind; mime: string; duration_ms: number; waveform: number[] | null; data_base64: string };
    const now = Math.floor(Date.now() / 1000);
    const m: MessengerMessage = { id: `local:${Date.now().toString(16)}`, chat_id: c.id, direction: "out", status: "sent", content_type: "media", text: null, sender_pubkey: "ab".repeat(32), reply_to: null, created_at: now, edited_at: null, deleted: false, failure_reason: null, media: { name: `${r.kind}.webm`, mime: r.mime, size: Math.round((r.data_base64.length * 3) / 4), kind: r.kind, duration_ms: r.duration_ms, waveform: r.waveform ?? undefined, local_path: "/dev/mock", mock_data: `data:${r.mime.split(";")[0]};base64,${r.data_base64}` } };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    return m;
  },
  messenger_media_download: () => null,
  messenger_media_transfer: () => null,
  messenger_media_pause: () => undefined,
  messenger_media_resume: () => undefined,
  messenger_media_cancel: () => undefined,
  messenger_media_save_as: () => undefined,
  messenger_media_data_url: (a) => (mockFind(String(a?.messageId))?.media?.mock_data as string | undefined) ?? null,
  messenger_media_local_path: () => null,
  messenger_media_open: () => undefined,
  messenger_open_url: (a) => { window.open(String(a?.url), '_blank', 'noopener'); },
  messenger_links_inspect: (a) => ((a?.links ?? []) as string[]).map((l) => mockInspect(l)),
  messenger_contact_link: (a) => {
    const c = mockContacts.find((x) => x.pubkey === a?.pubkey);
    return `veydan://contact/${c?.npub ?? `npub1${String(a?.pubkey).slice(0, 58)}`}${c ? `?n=${encodeURIComponent(contactLabel(c))}` : ''}`;
  },
  messenger_link_preview: (a) => {
    const host = new URL(String(a?.url)).hostname;
    if (host.startsWith('silent.')) throw new Error('preview_silent');
    if (host.startsWith('empty.')) throw new Error('preview_empty');
    return { url: String(a?.url), host, title: `${host}: заголовок страницы`, description: 'Описание, которое страница дала о себе. В приложении его читает Rust, и только по кнопке.', site_name: null, image: null };
  },
  messenger_dm_relation: (a) => {
    const c = mockChat(String(a?.peer));
    return { peer_pubkey: c.peer_pubkey, mode: c.mode, my_contact: 'approved', blocked: c.mode === 'blocked', peer_signal: 'approved', was_ever_mutual: true, can_send: c.can_send };
  },
  messenger_dm_action: (a) => {
    const c = mockChat(String(a?.peer));
    const next: Record<string, [ChatMode, boolean]> = {
      request: ['request_sent', false], accept: ['full_chat', true], decline: ['request_declined_by_me', false],
      block: ['blocked', false], unblock: ['full_chat', true], remove: ['mutual_reconnect', true],
    };
    const [mode, can_send] = next[String(a?.action)] ?? ['full_chat', true];
    mockChats = mockChats.map((x) => (x.id === c.id ? { ...x, mode, can_send } : x));
    return { peer_pubkey: c.peer_pubkey, mode, my_contact: 'approved', blocked: mode === 'blocked', peer_signal: 'approved', was_ever_mutual: true, can_send };
  },
  messenger_groups_list: () => mockGroups,
  messenger_group_get: (a) => mockGroup(String(a?.groupId)),
  messenger_group_create: (a) => {
    const id = Array.from({ length: 64 }, () => Math.floor(Math.random() * 16).toString(16)).join('');
    const kind = a?.kind === 'public' ? 'public' : 'private';
    const g: MessengerGroup = {
      id, chat_id: `group:${id}`, kind, name: String(a?.name), about: String(a?.about ?? ''), picture: '', relay: 'wss://relay.example',
      owner: 'ab'.repeat(32), membership: 'joined', my_role: 'owner', muted: false, can_post: true, history_for_new: kind === 'public' || Boolean(a?.historyForNew),
      members: [{ pubkey: 'ab'.repeat(32), role: 'owner', muted: false, joined_at: Math.floor(Date.now() / 1000), is_me: true }],
      banned: [], requests: [], undecrypted: 0,
      key: { id: id.slice(0, 32), version: 1, cipher: 'AES-256-GCM', source: kind === 'public' ? 'link' : 'random', link_epoch: 0, since: Math.floor(Date.now() / 1000), by: 'ab'.repeat(32), reason: 'create', held: true, status: 'good' },
      link: `veydan://group/${id}?t=${kind}&r=wss%3A%2F%2Frelay.example&o=${'ab'.repeat(32)}&n=${encodeURIComponent(String(a?.name))}${kind === 'public' ? '&s=demo&e=0' : ''}`,
    };
    mockGroups = [g, ...mockGroups];
    mockChats = [{ id: g.chat_id, kind: 'group', peer_pubkey: null, peer_npub: null, title: g.name, picture: null, is_contact: false, is_muted: false, unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false, mode: 'group', can_send: true }, ...mockChats];
    mockMessages[g.chat_id] = [];
    return g;
  },
  messenger_group_invite: (a) => ({ invite_id: 'demo', group_id: String(a?.groupId), name: mockGroup(String(a?.groupId)).name, about: '', picture: '', members: 1, peer: String(a?.who), direction: 'out', status: 'sent', created_at: Date.now() / 1000, expires_at: Date.now() / 1000 + 7 * 86400 }),
  messenger_group_invites: (a) => (a?.direction === 'in' ? mockInvites : []),
  messenger_group_answer_invite: (a) => { mockInvites = mockInvites.filter((i) => i.invite_id !== a?.inviteId); },
  messenger_group_open_link: (a) => {
    const v = mockInspect(String(a?.link));
    if (v.kind !== 'group') throw new Error('group_invalid');
    const known = mockGroups.find((x) => x.id === v.group_id);
    if (known?.membership === 'joined') throw new Error('group_already_member');
    const open = v.group_kind === 'public';
    const g: MessengerGroup = {
      id: v.group_id, chat_id: `group:${v.group_id}`, kind: v.group_kind, name: v.name, about: '', picture: '', relay: v.relay, owner: v.owner,
      membership: open ? 'joined' : 'requested', my_role: open ? 'member' : null, muted: false, can_post: open, history_for_new: true,
      members: open ? [{ pubkey: 'ab'.repeat(32), role: 'member', muted: false, joined_at: Math.floor(Date.now() / 1000), is_me: true }] : [],
      banned: [], requests: [], link: null, undecrypted: 0, key: null,
    };
    mockGroups = [g, ...mockGroups.filter((x) => x.id !== g.id)];
    if (!mockChats.some((c) => c.id === g.chat_id)) {
      mockChats = [{ id: g.chat_id, kind: 'group', peer_pubkey: null, peer_npub: null, title: g.name, picture: null, is_contact: false, is_muted: false, unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false, mode: 'group', can_send: open }, ...mockChats];
      mockMessages[g.chat_id] = [];
    }
    return g;
  },
  messenger_group_answer_request: (a) => {
    const g = mockGroup(String(a?.groupId));
    g.requests = g.requests.filter((r) => r !== a?.requester);
    if (a?.approve) g.members = [...g.members, { pubkey: String(a?.requester), role: 'member', muted: false, joined_at: Math.floor(Date.now() / 1000), is_me: false }];
    return g;
  },
  messenger_group_act: (a) => {
    const g = mockGroup(String(a?.groupId));
    const act = (a?.action ?? {}) as GroupAction;
    switch (act.op) {
      case 'remove': g.members = g.members.filter((m) => m.pubkey !== act.who); break;
      case 'ban': g.members = g.members.filter((m) => m.pubkey !== act.who); g.banned = [...g.banned, act.who]; break;
      case 'unban': g.banned = g.banned.filter((b) => b !== act.who); break;
      case 'set_role': g.members = g.members.map((m) => (m.pubkey === act.who ? { ...m, role: act.role } : m)); break;
      case 'set_muted': g.members = g.members.map((m) => (m.pubkey === act.who ? { ...m, muted: act.muted } : m)); break;
      case 'edit_settings':
        if (act.name) g.name = act.name;
        if (act.about !== undefined) g.about = act.about;
        if (act.history_for_new !== undefined) g.history_for_new = act.history_for_new;
        mockChats = mockChats.map((c) => (c.id === g.chat_id ? { ...c, title: g.name } : c));
        break;
      case 'leave': g.membership = 'left'; g.can_post = false; g.my_role = null; break;
      case 'disband': g.membership = 'disbanded'; g.can_post = false; break;
      case 'transfer_ownership':
        g.members = g.members.map((m) => (m.pubkey === act.to ? { ...m, role: 'owner' } : m.is_me ? { ...m, role: 'admin' } : m));
        g.my_role = 'admin';
        break;
    }
    if (g.membership !== 'joined') mockChats = mockChats.map((c) => (c.id === g.chat_id ? { ...c, can_send: false } : c));
    return g;
  },
  messenger_group_rotate_link: (a) => {
    const g = mockGroup(String(a?.groupId));
    const e = Number(/&e=(\d+)/.exec(g.link ?? '')?.[1] ?? 0) + 1;
    g.link = (g.link ?? '').replace(/&e=\d+/, `&e=${e}`).replace(/&s=[^&]+/, `&s=demo${e}`);
    return g;
  },
  messenger_group_link_qr: () =>
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29" shape-rendering="crispEdges"><rect width="29" height="29" fill="#fff"/><path fill="#000" d="M4 4h7v7H4zM18 4h7v7h-7zM4 18h7v7H4zM6 6v3h3V6zM20 6v3h3V6zM6 20v3h3v-3zM13 4h2v2h-2zM13 8h3v2h-3zM12 12h2v2h-2zM16 13h3v2h-3zM21 13h4v2h-4zM4 13h5v2H4zM13 17h2v3h-2zM17 18h2v2h-2zM21 17h2v2h-2zM18 22h3v3h-3zM23 21h2v2h-2zM13 22h3v2h-3z"/></svg>',
  messenger_group_forget: (a) => {
    mockGroups = mockGroups.filter((g) => g.id !== a?.groupId);
    mockChats = mockChats.filter((c) => c.id !== `group:${a?.groupId}`);
  },
  messenger_dm_blocked: () => mockChats.filter((c) => c.mode === 'blocked').map((c) => c.peer_pubkey),
  /** Dev only: put the open mock chat into a given screen mode. */
  messenger_dev_set_mode: (a) => {
    mockChats = mockChats.map((x) => (x.id === a?.chatId ? { ...x, mode: a?.mode as ChatMode, can_send: Boolean(a?.canSend) } : x));
  },
  messenger_profile_get: () => null,
  messenger_profile_request: () => undefined,
  messenger_profile_own_get: () => mockOwnProfile,
  messenger_profile_own_set: (a) => {
    const i = (a?.input ?? {}) as MessengerProfileInput;
    mockOwnProfile = { ...(mockOwnProfile ?? emptyProfile('ab'.repeat(32))), ...i, event_created_at: Date.now() / 1000 } as MessengerProfile;
    return mockOwnProfile;
  },
  messenger_nip05_verify: () => false,
  messenger_contacts_list: () => mockContacts,
  messenger_contacts_add: (a) => {
    const c: MessengerContact = { pubkey: 'ef'.repeat(32), npub: 'npub1mockcontactmockcontactmockcontactmockcontactmockcontact0000', nickname: (a?.nickname as string) ?? null, note: null, followed: false, profile: null, created_at: Date.now() / 1000, updated_at: Date.now() / 1000 };
    mockContacts = [c, ...mockContacts];
    return c;
  },
  messenger_contacts_update: (a) => { mockContacts = mockContacts.map((c) => c.pubkey === a?.pubkey ? { ...c, ...(a?.patch as object) } : c); return mockContacts.find((c) => c.pubkey === a?.pubkey); },
  messenger_contacts_remove: (a) => { mockContacts = mockContacts.filter((c) => c.pubkey !== a?.pubkey); },
  messenger_contacts_set_followed: (a) => { mockContacts = mockContacts.map((c) => c.pubkey === a?.pubkey ? { ...c, followed: Boolean(a?.followed) } : c); },
  messenger_relays_list: () => (mockServersMode ? mockRelays : mockRelays.filter((r) => r.source === 'user')),
  messenger_relays_add: (a) => {
    const r: MessengerRelay = { url: String(a?.url), relay_id: null, source: 'user', regions: [], read: true, write: true, enabled: true, auth_type: null, state: 'connecting' };
    mockRelays = [...mockRelays, r];
    return r;
  },
  messenger_relays_remove: (a) => { mockRelays = mockRelays.filter((r) => r.url !== a?.url); },
  messenger_relays_set_enabled: (a) => { mockRelays = mockRelays.map((r) => r.url === a?.url ? { ...r, enabled: Boolean(a?.enabled) } : r); },
  messenger_relays_set_silent: (a) => { mockSilent = Boolean(a?.enabled); },
  messenger_manifest_info: () => ({
    serial: mockServersMode === 'veydan' ? (mockManifestOk ? 5 : 4) : null, issued_at: 1790726400, region: mockRegion, regions: ['default', 'ru'],
    silent_mode: mockSilent, mode: mockServersMode, origin: mockManifestOrigin, checked_at: mockCheckedAt,
  }),
  messenger_servers_use_veydan: async () => {
    await new Promise((r) => setTimeout(r, 700));
    mockServersMode = 'veydan';
    if (!mockRelays.some((r) => r.source === 'manifest')) mockRelays = [manifestRelay(), ...mockRelays];
    return mockManifestCheck();
  },
  messenger_servers_use_own: () => {
    if (!mockRelays.some((r) => r.source === 'user' && r.enabled)) throw new Error('servers_own_needs_relay');
    mockServersMode = 'own';
    mockRelays = mockRelays.filter((r) => r.source === 'user');
    mockMediaServers = mockMediaServers.filter((s) => s.source === 'user');
  },
  messenger_manifest_refresh: () => (mockServersMode === 'veydan' ? mockManifestCheck() : null),
  messenger_manifest_set_region: (a) => { mockRegion = String(a?.region); },
  messenger_net_status: () => mockNetApply(),
  messenger_net_set_mode: (a) => {
    mockNet = { ...mockNet, mode: a?.mode as NetMode, offer: false, list_checked_at: Math.floor(Date.now() / 1000) };
    return mockNetApply();
  },
  messenger_net_check: (): NetCheck => {
    if (mockRestricted && mockNet.mode === 'off') mockNet = { ...mockNet, offer: true };
    return mockRestricted ? { direct: false, bridge: true, verdict: 'restricted' } : { direct: true, bridge: false, verdict: 'direct' };
  },
  messenger_net_bridge_add: (a) => {
    const bridge = mockBridgeOf(String(a?.bridge));
    const other = /^veydan:\/\/([a-z]+)\//.exec(String(a?.bridge).trim())?.[1];
    if (!bridge) throw { code: 'other', message: other && other !== 'vlink' ? 'net_bridge_link_type' : 'net_bridge_invalid' };
    if (!mockNet.private.some((b) => b.id === bridge.id)) mockNet = { ...mockNet, private: [bridge, ...mockNet.private] };
    return mockNetApply();
  },
  messenger_net_bridge_remove: (a) => {
    mockNet = { ...mockNet, private: mockNet.private.filter((b) => b.id !== a?.id) };
    return mockNetApply();
  },
  messenger_net_offer_dismiss: () => { mockNet = { ...mockNet, offer: false }; },
  messenger_set_enabled: () => undefined,
  messenger_identity_get: () => mockIdentity,
  messenger_identity_create: () => {
    mockIdentity = { npub: 'npub1devdevdevdevdevdevdevdevdevdevdevdevdevdevdevdevdevdevdev0000', pubkey: 'ab'.repeat(32), created_at: Date.now() / 1000 };
    return { identity: mockIdentity, ncryptsec: 'ncryptsec1devbackupdevbackupdevbackup' };
  },
  messenger_identity_import: () => {
    mockIdentity = { npub: 'npub1importedimportedimportedimportedimportedimportedimported00', pubkey: 'cd'.repeat(32), created_at: Date.now() / 1000 };
    return mockIdentity;
  },
  messenger_identity_export: () => 'ncryptsec1devbackupdevbackupdevbackup',
  messenger_identity_delete: () => { mockIdentity = null; },
  // In the browser the phone is played: the panel can be looked at and clicked through.
  messenger_push_status: (): MessengerPushView => mockPush,
  messenger_push_set_enabled: (a): MessengerPushView => {
    const on = Boolean(a?.enabled);
    const now = Math.floor(Date.now() / 1000);
    mockPush = {
      device: { ...mockPush.device, permission: on ? 'granted' : mockPush.device.permission },
      status: {
        ...mockPush.status, enabled: on, offered: true,
        state: on ? 'registered' : 'off',
        last_ok_at: on ? now : null,
        expires_at: on ? now + 30 * 86400 : null,
        relays: on
          ? [
              { url: 'wss://node-1.veydan.net', status: 'ok' },
              { url: 'wss://relay.example.org', status: 'not_allowed', detail: 'this server does not watch this relay' },
            ]
          : [],
      },
    };
    return mockPush;
  },
  messenger_push_mark_offered: () => { mockPush = { ...mockPush, status: { ...mockPush.status, offered: true } }; },
  messenger_push_set_server: (a): MessengerPushView => {
    const url = a?.url ? String(a.url) : null;
    mockPush = { ...mockPush, status: { ...mockPush.status, server: url ?? 'https://vpush.veydan.net', server_custom: Boolean(url) } };
    return mockPush;
  },
  messenger_push_set_prefs: (a): MessengerPushView => {
    mockPush = { ...mockPush, status: { ...mockPush.status, dm: Boolean(a?.dm), groups: Boolean(a?.groups) } };
    return mockPush;
  },
  messenger_push_refresh: (): MessengerPushView => mockPush,
  messenger_push_test: (): MessengerPushTest => ({ outcome: 'delivered', trace: '5f3a9c1e' }),
  messenger_push_take_tap: () => null,
  messenger_push_clear: () => undefined,
  messenger_notify_get: () => mockNotify,
  messenger_desktop_notify_get: () => mockDesktopNotify,
  messenger_desktop_notify_set: (a) => { mockDesktopNotify = { ...mockDesktopNotify, enabled: Boolean(a?.enabled), sound: Boolean(a?.sound) }; return mockDesktopNotify; },
  messenger_desktop_notify_test: () => undefined,
  messenger_notice_words: () => undefined,
  messenger_desktop_notify_keep_running: () => { mockDesktopNotify = { ...mockDesktopNotify, close_to_tray: true }; return mockDesktopNotify; },
  messenger_notify_set: (a) => { mockNotify = { ...mockNotify, content: a?.content as MessengerNotifySettings['content'], lockscreen_hidden: Boolean(a?.lockscreenHidden) }; return mockNotify; },
};

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }
  console.warn(`[dev-browser] messenger invoke('${cmd}')`, args ?? '');
  return devMocks[cmd]?.(args) as T;
}

/** True when Tauri reports the command does not exist (feature not compiled). */
function isUnknownCommand(e: unknown): boolean {
  const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e ?? '');
  return /command .* not found/i.test(msg) || /not allowed/i.test(msg);
}

/** Human-readable text for a host error or anything else thrown. */
export function messengerError(e: unknown): string {
  if (e != null && typeof e === 'object' && 'code' in e) {
    const err = e as MessengerError;
    return err.message ?? err.code;
  }
  return typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
}

export function messengerErrorCode(e: unknown): string | null {
  if (e != null && typeof e === 'object' && 'code' in e) return String((e as MessengerError).code);
  return null;
}

export const messengerApi = {
  async status(): Promise<MessengerStatus> {
    try {
      return await invoke<MessengerStatus>('messenger_status');
    } catch (e) {
      if (isUnknownCommand(e)) return NOT_COMPILED;
      throw e;
    }
  },
  /** Open an http(s) link in the system browser. */
  openUrl: (url: ExternalUrl) => invoke<void>('messenger_open_url', { url }),

  links: {
    /** One answer per link, in the order asked. Asks nothing of the network. */
    inspect: (links: InternalLinkText[]) => invoke<LinkView[]>('messenger_links_inspect', { links }),
    /** The link of a person, to share. */
    contact: (pubkey: string) => invoke<string>('messenger_contact_link', { pubkey }),
    /** Asks the page: only when the user said so. */
    preview: (url: ExternalUrl) => invoke<LinkPreview>('messenger_link_preview', { url }),
  },

  push: {
    /** Asks nothing of the push service or the push server. */
    status: () => invoke<MessengerPushView>('messenger_push_status'),
    /**
     * On: the permission is asked for, then the phone's address at the push
     * service, then the push server is told. What failed is in the answer.
     * Off: the registration is taken back.
     */
    setEnabled: (enabled: boolean) => invoke<MessengerPushView>('messenger_push_set_enabled', { enabled }),
    /** The user was asked and said "not now". */
    markOffered: () => invoke<void>('messenger_push_mark_offered'),
    /** Nothing goes back to the server of the manifest. */
    setServer: (url: string | null) => invoke<MessengerPushView>('messenger_push_set_server', { url }),
    setPrefs: (dm: boolean, groups: boolean) => invoke<MessengerPushView>('messenger_push_set_prefs', { dm, groups }),
    refresh: () => invoke<MessengerPushView>('messenger_push_refresh'),
    test: () => invoke<MessengerPushTest>('messenger_push_test'),
    takeTap: () => invoke<MessengerPushTap | null>('messenger_push_take_tap'),
    notify: () => invoke<MessengerNotifySettings>('messenger_notify_get'),
    setNotify: (content: MessengerNotifySettings['content'], lockscreenHidden: boolean) =>
      invoke<MessengerNotifySettings>('messenger_notify_set', { content, lockscreenHidden }),
    /** `dm`, `group:<id>`, or nothing for every notification about messages. */
    clear: (key?: string) => invoke<void>('messenger_push_clear', { key: key ?? null }),
  },

  /** A computer's own notifications (no push: the app runs and shows them). */
  desktopNotify: {
    get: () => invoke<MessengerDesktopNotify>('messenger_desktop_notify_get'),
    set: (enabled: boolean, sound: boolean) => invoke<MessengerDesktopNotify>('messenger_desktop_notify_set', { enabled, sound }),
    test: (title: string, body: string) => invoke<void>('messenger_desktop_notify_test', { title, body }),
    words: (words: MessengerNoticeWords) => invoke<void>('messenger_notice_words', { words }),
    /** Closing the window keeps the app in the tray, so notifications keep coming. */
    keepRunning: () => invoke<MessengerDesktopNotify>('messenger_desktop_notify_keep_running'),
  },

  relays: {
    list: () => invoke<MessengerRelay[]>('messenger_relays_list'),
    add: (url: string, apiKey?: string) =>
      invoke<MessengerRelay>('messenger_relays_add', { url, apiKey: apiKey && apiKey.trim() ? apiKey.trim() : null }),
    remove: (url: string) => invoke<void>('messenger_relays_remove', { url }),
    setEnabled: (url: string, enabled: boolean) => invoke<void>('messenger_relays_set_enabled', { url, enabled }),
    setSilent: (enabled: boolean) => invoke<void>('messenger_relays_set_silent', { enabled }),
    manifestInfo: () => invoke<MessengerManifestInfo>('messenger_manifest_info'),
    setRegion: (region: string) => invoke<void>('messenger_manifest_set_region', { region }),
    useVeydan: () => invoke<MessengerManifestCheck>('messenger_servers_use_veydan'),
    useOwn: () => invoke<void>('messenger_servers_use_own'),
    refreshManifest: () => invoke<MessengerManifestCheck | null>('messenger_manifest_refresh'),
  },

  /** The way to the project's servers: directly, or through a bridge. */
  net: {
    status: () => invoke<NetStatus>('messenger_net_status'),
    setMode: (mode: NetMode) => invoke<NetStatus>('messenger_net_set_mode', { mode }),
    /** Tries the direct way and a bridge now. */
    check: () => invoke<NetCheck>('messenger_net_check'),
    /** A link (`veydan://vlink/…`) or a reference (`address:port#id`). */
    addBridge: (bridge: string) => invoke<NetStatus>('messenger_net_bridge_add', { bridge }),
    removeBridge: (id: string) => invoke<NetStatus>('messenger_net_bridge_remove', { id }),
    dismissOffer: () => invoke<void>('messenger_net_offer_dismiss'),
  },

  chats: {
    list: (includeArchived = false) => invoke<MessengerChat[]>('messenger_chats_list', { includeArchived }),
    open: (peer: string) => invoke<MessengerChat>('messenger_chat_open', { peer }),
    messages: (chatId: string, before?: number, limit = 50) =>
      invoke<MessengerMessage[]>('messenger_chat_messages', { chatId, before: before ?? null, limit }),
    /** How many messages each section of what the chat has shared holds. */
    sharedCounts: (chatId: string) => invoke<SharedCounts>('messenger_chat_shared_counts', { chatId }),
    /** One page of a section, newest first, strictly older than `before`. */
    shared: (chatId: string, section: SharedSection, before?: number, limit = 60) =>
      invoke<MessengerMessage[]>('messenger_chat_shared', { chatId, section, before: before ?? null, limit }),
    markRead: (chatId: string) => invoke<void>('messenger_chat_mark_read', { chatId }),
    setPinned: (chatId: string, pinned: boolean) => invoke<void>('messenger_chat_set_pinned', { chatId, pinned }),
    setArchived: (chatId: string, archived: boolean) => invoke<void>('messenger_chat_set_archived', { chatId, archived }),
    /** Direct chats and groups alike: a muted chat is still counted and shown, only quietly. */
    setMuted: (chatId: string, muted: boolean) => invoke<void>('messenger_chat_set_muted', { chatId, muted }),
    delete: (chatId: string) => invoke<void>('messenger_chat_delete', { chatId }),
  },

  dm: {
    sendText: (to: string, text: string, replyTo?: string) =>
      invoke<MessengerMessage>('messenger_dm_send_text', { to, text, replyTo: replyTo ?? null }),
    edit: (messageId: string, text: string) => invoke<MessengerMessage>('messenger_dm_edit', { messageId, text }),
    delete: (messageId: string, forEveryone: boolean) => invoke<void>('messenger_dm_delete', { messageId, forEveryone }),
    retry: (messageId: string) => invoke<void>('messenger_dm_retry', { messageId }),
    relation: (peer: string) => invoke<MessengerRelation>('messenger_dm_relation', { peer }),
    action: (peer: string, action: DmAction) => invoke<MessengerRelation>('messenger_dm_action', { peer, action }),
    blocked: () => invoke<string[]>('messenger_dm_blocked'),
  },


  groups: {
    list: () => invoke<MessengerGroup[]>('messenger_groups_list'),
    get: (groupId: string) => invoke<MessengerGroup>('messenger_group_get', { groupId }),
    create: (kind: 'public' | 'private', name: string, about: string, historyForNew: boolean) =>
      invoke<MessengerGroup>('messenger_group_create', { kind, name, about, historyForNew }),
    invite: (groupId: string, who: string) => invoke<MessengerGroupInvite>('messenger_group_invite', { groupId, who }),
    invites: (direction: 'in' | 'out') => invoke<MessengerGroupInvite[]>('messenger_group_invites', { direction }),
    answerInvite: (inviteId: string, accept: boolean) => invoke<void>('messenger_group_answer_invite', { inviteId, accept }),
    openLink: (link: string, note?: string) => invoke<MessengerGroup>('messenger_group_open_link', { link, note: note ?? null }),
    answerRequest: (groupId: string, requester: string, approve: boolean) =>
      invoke<MessengerGroup>('messenger_group_answer_request', { groupId, requester, approve }),
    act: (groupId: string, action: GroupAction) => invoke<MessengerGroup>('messenger_group_act', { groupId, action }),
    rotateLink: (groupId: string) => invoke<MessengerGroup>('messenger_group_rotate_link', { groupId }),
    /** SVG of the QR code of the link. */
    linkQr: (groupId: string) => invoke<string>('messenger_group_link_qr', { groupId }),
    forget: (groupId: string) => invoke<void>('messenger_group_forget', { groupId }),
  },

  media: {
    servers: () => invoke<MessengerMediaServer[]>('messenger_media_servers'),
    putServer: (input: MessengerMediaServerInput) => invoke<MessengerMediaServer>('messenger_media_server_put', { input }),
    removeServer: (id: string) => invoke<void>('messenger_media_server_remove', { id }),
    setServerEnabled: (id: string, enabled: boolean) => invoke<void>('messenger_media_server_set_enabled', { id, enabled }),
    checkServer: (id: string) => invoke<void>('messenger_media_server_check', { id }),
    sendRecording: async (to: string, rec: MessengerRecording) =>
      invoke<MessengerMessage>('messenger_dm_send_recording', {
        to,
        recording: {
          kind: rec.kind, mime: rec.mime, duration_ms: Math.round(rec.duration_ms),
          waveform: rec.waveform ?? null, data_base64: await blobToBase64(rec.blob),
        },
      }),
    /** Must run before the first `getUserMedia` (desktop webviews deny otherwise). */
    grantAccess: () => invoke<void>('messenger_media_grant_access'),
    /** `batch`: the same for files picked together. */
    sendFile: (to: string, path: string, caption?: string, batch?: string) =>
      invoke<MessengerMessage>('messenger_dm_send_file', { to, path, caption: caption?.trim() || null, batch: batch ?? null }),
    download: (messageId: string, manual: boolean) => invoke<string | null>('messenger_media_download', { messageId, manual }),
    transfer: (messageId: string) => invoke<MessengerTransfer | null>('messenger_media_transfer', { messageId }),
    pause: (transferId: string) => invoke<void>('messenger_media_pause', { transferId }),
    resume: (transferId: string) => invoke<void>('messenger_media_resume', { transferId }),
    cancel: (transferId: string) => invoke<void>('messenger_media_cancel', { transferId }),
    saveAs: (messageId: string, dest: string) => invoke<void>('messenger_media_save_as', { messageId, dest }),
    dataUrl: (messageId: string) => invoke<string | null>('messenger_media_data_url', { messageId }),
    localPath: (messageId: string) => invoke<string | null>('messenger_media_local_path', { messageId }),
    /** Opens with the default application; runnable files are only revealed in their folder. */
    open: (messageId: string) => invoke<void>('messenger_media_open', { messageId }),
  },

  profiles: {
    get: (pubkey: string) => invoke<MessengerProfile | null>('messenger_profile_get', { pubkey }),
    request: (pubkey: string) => invoke<void>('messenger_profile_request', { pubkey }),
    ownGet: () => invoke<MessengerProfile | null>('messenger_profile_own_get'),
    ownSet: (input: MessengerProfileInput) => invoke<MessengerProfile>('messenger_profile_own_set', { input }),
    verifyNip05: (pubkey: string) => invoke<boolean>('messenger_nip05_verify', { pubkey }),
  },

  contacts: {
    list: () => invoke<MessengerContact[]>('messenger_contacts_list'),
    add: (key: string, nickname?: string) =>
      invoke<MessengerContact>('messenger_contacts_add', { key, nickname: nickname?.trim() || null }),
    update: (pubkey: string, patch: MessengerContactPatch) =>
      invoke<MessengerContact>('messenger_contacts_update', { pubkey, patch }),
    remove: (pubkey: string) => invoke<void>('messenger_contacts_remove', { pubkey }),
    setFollowed: (pubkey: string, followed: boolean) =>
      invoke<void>('messenger_contacts_set_followed', { pubkey, followed }),
  },

  identity: {
    get: () => invoke<MessengerIdentity | null>('messenger_identity_get'),
    create: (password: string) => invoke<MessengerCreatedIdentity>('messenger_identity_create', { password }),
    import: (kind: IdentityImportKind, secret: string, password?: string) =>
      invoke<MessengerIdentity>('messenger_identity_import', { kind, secret, password: password ?? null }),
    export: (password: string) => invoke<string>('messenger_identity_export', { password }),
    delete: () => invoke<void>('messenger_identity_delete'),
  },
};
export const isTauriHost = isTauri;
