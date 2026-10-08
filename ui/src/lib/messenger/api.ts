// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Messenger command layer. Intentionally separate from `$lib/api` so the
// module can be lifted into a standalone app: this file is the only place
// that knows command names (internal/messenger-spec.md §4.6).

// Types the runtime writes for itself (`make msg-types`); never by hand.
export type { GroupMembership, LinkGroupKind, LinkPreview, LinkView } from './generated/links';
export type { SharedCounts, SharedSection } from './generated/shared';
export type { BridgeView, NetCheck, NetMode, NetStatus, Verdict } from './generated/net';
import type { NetCheck, NetMode, NetStatus } from './generated/net';
export type {
  CallDirection, CallEnded, CallLimits, CallMedia, CallNodeInput, CallNodeView, CallOutcome, CallPhase, CallState, CallStats, CallView, CallVia, RelayPolicy,
  CameraInfo, ScreenInfo, VideoInput, VideoQuality, VideoSize, VideoTrack,
  GroupCallAnnounced, GroupCallEnded, GroupCallLevel, GroupCallPhase, GroupCallState, GroupCallView, GroupParticipant,
} from './generated/calls';
import type {
  CallMedia, CallNodeInput, CallState, CallView, CameraInfo, GroupCallState, GroupCallView, RelayPolicy, ScreenInfo, VideoInput, VideoQuality, VideoTrack,
} from './generated/calls';

/**
 * The state of calls as the runtime says it. `incoming_enabled` (calls ring
 * on this device) comes with a runtime that has `messenger_call_set_incoming`;
 * a runtime without it says nothing, and the settings do not offer it.
 */
export type CallStateView = CallState & { incoming_enabled?: boolean };

export interface MessengerIngressCounters {
  received: number;
  duplicates: number;
  dispatched: number;
  dm: number;
  ignored: number;
}

export type MessengerLink = 'ok' | 'waiting' | 'lost';

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
  /** What the app shows about its connection: connected from the start and
   *  after a wake, `waiting` after 10 s without relays, `lost` after 60 s. */
  link: MessengerLink;
  manifest_serial: number | null;
  region: string;
  /** Whose servers are used; `null` until the user chooses (onboarding). */
  servers_mode: MessengerServersMode | null;
  ingress: MessengerIngressCounters;
  outbox_pending: number;
}

// Profiles, avatars and contact cards (the runtime's types, as generated).
export type {
  AvatarPreview, CardView, Color, ContactPrivateView, CropRect, OwnPrivateView, ProfileInput, ProfileView,
  SocialLink, SocialPlatform, SocialView, Span, Style,
} from './generated/profile';
import type {
  AvatarPreview, CardView, Color, ContactPrivateView, CropRect, OwnPrivateView, ProfileInput, ProfileView,
  SocialLink, SocialPlatform, SocialView, Span, Style,
} from './generated/profile';

/** A profile as the runtime shows it: bio as spans, socials checked, `picture` set by the runtime alone. */
export type MessengerProfile = ProfileView;

/** What the user edits of their own profile; there is no picture: the avatar has commands of its own. */
export type MessengerProfileInput = ProfileInput;

/** An input with every field of `p` as it is, for an editor to start from. */
export function profileInputOf(p: MessengerProfile | null): MessengerProfileInput {
  return {
    name: p?.name ?? null, display_name: p?.display_name ?? null, about: p?.bio_source ?? p?.about ?? null,
    website: p?.website ?? null, nip05: p?.nip05 ?? null, lud16: p?.lud16 ?? null,
    socials: (p?.socials ?? []).map(socialLinkOf),
  };
}

/** A shown link back in the form the runtime takes (a handle as written there; "other" by its address). */
export function socialLinkOf(v: SocialView): SocialLink {
  return { p: v.platform, h: v.platform === 'other' ? v.url : v.handle };
}

/** The colours of a bio, in the order a palette shows them. */
export const BIO_COLORS: readonly Color[] = ['red', 'orange', 'yellow', 'green', 'teal', 'blue', 'purple', 'pink', 'gray'];

/** The bio's limit, as the runtime counts it (Unicode scalar values). */
export const BIO_MAX_CHARS = 2000;

/** At most this many social links on a profile. */
export const SOCIALS_MAX = 16;

/** Codes of refusals about profiles, avatars, phones and cards. */
const PROFILE_CODES = /\b(bio_too_long|social_unknown_platform|social_bad_handle|profile_too_large|phone_invalid|phone_public_group|card_invalid|card_too_large|card_unknown|card_is_me|avatar_too_large|avatar_unsupported|avatar_corrupt|avatar_bad_crop|avatar_expired|avatar_not_ours|avatar_unavailable)\b|\berr\.(media_no_server)\b/;

/**
 * The stable code of a refusal about a profile, an avatar, a phone or a card
 * (`bio_too_long`, `avatar_bad_crop`, `media_no_server` for
 * `err.media_no_server`…), if any. The UI shows `msg_err_<code>`.
 */
export function profileErrorCode(e: unknown): string | null {
  const m = PROFILE_CODES.exec(typeof e === 'string' ? e : messengerError(e));
  return m ? (m[1] ?? m[2]) : null;
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
  /** While `queued`: when it went to the outbox (unix seconds). */
  queued_at?: number | null;
  /** Outgoing only: when a device of the peer had it (unix seconds). */
  delivered_at: number | null;
  /** Outgoing only: when the peer, or any member of a group, read it. */
  read_at: number | null;
  /** Group members who read this outgoing message; empty elsewhere. */
  seen_by: string[];
  /** Reactions on it, in the order they first came; empty when deleted. */
  reactions: MessengerReaction[];
  /** A contact card (`content_type` `contact`), as the runtime checked it; its `media` is then empty. */
  card?: CardView | null;
}

/** One emoji on a message: how many put it, and whether I am among them. */
export interface MessengerReaction {
  emoji: string;
  count: number;
  mine: boolean;
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
  // Refusals of a reaction come as their own codes, not under `dm_`.
  const m = /\b(?:dm_[a-z_]+|reaction_(?:limit|invalid))\b/.exec(messengerError(e));
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
  /** Encrypted parts the file is stored in, when the message lists them. */
  chunks?: number;
  /** Recordings: length and loudness outline (0..255 per bar). */
  duration_ms?: number;
  waveform?: number[];
  /** A small JPEG (base64) of the picture or of a frame of the video, shown before the file is here. */
  thumb?: string;
  /** Width and height the picture or the video is shown at. */
  dim?: [number, number];
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
    chunks: Array.isArray(f.chunks) && f.chunks.length ? f.chunks.length : undefined,
    duration_ms: typeof f.duration_ms === "number" ? f.duration_ms : undefined,
    waveform: Array.isArray(f.waveform) ? (f.waveform as unknown[]).filter((x): x is number => typeof x === "number") : undefined,
    thumb: typeof f.thumb === "string" && f.thumb ? f.thumb : undefined,
    dim: dimOf(f.dim),
  };
}

function dimOf(v: unknown): [number, number] | undefined {
  if (!Array.isArray(v) || v.length !== 2) return undefined;
  const [w, h] = v;
  return typeof w === "number" && typeof h === "number" && w > 0 && h > 0 ? [w, h] : undefined;
}

// File transfers (the runtime's types, as generated).
export type { TransferStage } from './generated/transfer';
import type { Progress, TransferStage, TransferView } from './generated/transfer';

/** queued | running | waiting_retry | paused | done | failed | cancelled */
export type TransferStatus = TransferView['status'];

/** A transfer as the runtime keeps it (`messenger_media_transfer`, `messenger_media_transfers`). */
export type MessengerTransfer = TransferView;

/** Payload of the `transfer.progress` runtime event. */
export type MessengerTransferProgress = Progress;

/** The largest file that is sent; the runtime refuses more with `err.file_too_large`. */
export const MAX_SEND_BYTES = 1024 ** 3;

/** A kept transfer as its last event would have told it. */
export function transferProgress(v: MessengerTransfer): MessengerTransferProgress {
  return {
    transfer_id: v.id, message_id: v.message_id, chat_id: v.chat_id, direction: v.direction, status: v.status,
    done_bytes: v.done_bytes, total_bytes: v.size, failure_reason: v.failure_reason, local_path: v.local_path,
    stage: v.stage ?? (v.direction === 'up' ? 'uploading' : 'downloading'), chunks_done: v.chunks_done ?? 0,
    chunks_total: v.chunks_total ?? 0, chunk_size: v.chunk_size ?? 0, rate_bps: v.rate_bps ?? 0, eta_secs: v.eta_secs ?? null,
    retry_at_ms: v.retry_at_ms ?? null, attempt: v.attempts, file_name: v.file_name, mime: v.mime,
  };
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

/** A picked file waiting in the composer until it is sent. */
export interface MessengerPicked {
  /** On this device; what is sent. */
  path: string;
  name: string;
  kind: MediaKind;
  /** The type it is sent as, by its name. */
  mime?: string;
  size: number;
  /** A picture as a `data:` url; `null` for anything else. */
  preview: string | null;
  /** A video: where the webview reads it, for a frame of it. */
  url?: string | null;
}

/** A frame the UI took from a video it sends: a JPEG (base64, no `data:` prefix) and the size of the video. */
export interface MessengerPoster {
  jpeg: string;
  width: number;
  height: number;
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

/** What the other side learns of the user; each works both ways: off here, the user sees nothing of it from others either. */
export interface MessengerPrivacy {
  read_receipts: boolean;
  presence: boolean;
}

/** When an approved contact was last seen and until when they count as online (unix seconds). */
export interface MessengerPresence {
  /** The contact's own key (hex), not the key they announce presence from. */
  peer: string;
  seen_at: number;
  online_until: number;
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
  /** The body of a ringing call's notification. */
  call_audio: string;
  call_video: string;
  /** Its buttons. */
  call_answer: string;
  call_decline: string;
  /** The body of the quiet notification of a call on in a group (the title is the group's name). */
  group_call_audio: string;
  group_call_video: string;
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

/** Where the sound of a call goes on a phone (the call plugin's words). */
export type CallAudioRoute = 'earpiece' | 'speaker' | 'bluetooth' | 'wired';

/** The routes a phone has now; `current` is `null` while no call holds the sound. */
export interface CallAudioRoutes {
  current: CallAudioRoute | null;
  available: CallAudioRoute[];
}

export type IdentityImportKind = 'nsec' | 'ncryptsec' | 'mnemonic';

/** Error shape forwarded by the host (`{ code, message }`). */
export interface MessengerError {
  code: string;
  message?: string;
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const NOT_COMPILED: MessengerStatus = { compiled: false, enabled: false, runtime: null, error: null };

import {
  buildDemo, demoBioParse, demoCard, demoEnabled, demoPhone, demoPicture, demoSocialLink, demoSocialView, demoStrip,
  DEMO_PICTURE_BASE, DEMO_PLATFORMS,
} from './devDemo';
import type { ExternalUrl, InternalLinkText } from './content/types';
import type { GroupMembership, LinkPreview, LinkView } from './generated/links';
import type { SharedCounts, SharedSection } from './generated/shared';
import { inSection } from './content/shared/sections';
import { demoCallMocks } from './calls/demo';

const demo = !isTauri && demoEnabled() ? buildDemo() : null;
let mockIdentity: MessengerIdentity | null = demo?.identity ?? null;
function manifestRelay(): MessengerRelay {
  return { url: 'wss://node-1.veydan.net', relay_id: 'veydan-node-1', source: 'manifest', regions: ['default', 'ru'], read: true, write: true, enabled: true, auth_type: 'api_key', state: 'connected' };
}
let mockRelays: MessengerRelay[] = [manifestRelay()];
let mockNotify: MessengerNotifySettings = { content: 'sender_text', lockscreen_hidden: false, locked: false };
let mockPrivacy: MessengerPrivacy = { read_receipts: true, presence: true };
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
  bio: [], bio_source: null, socials: [],
});

// Avatars, private parts and cards in the browser preview. In the app Rust
// decodes, crops, uploads and fetches; here pictures are drawn on a canvas.
const mockAvatars = new Map<string, string | null>();
let mockAvatarPick: { token: string; preview: string } | null = null;
let mockOwnPrivate: OwnPrivateView = { phone: demo ? '+79991234567' : null, share_phone: true };
const mockContactPhones: Record<string, string> = demo ? { ['1a'.repeat(32)]: '+380671234567' } : {};

/** Pictures of the demo and the user's own are "cached"; of other URLs about two in three, the rest never come. */
function mockAvatarCached(url: string): string | null {
  if (!url.startsWith('https://')) throw new Error('avatar_unsupported');
  if (mockAvatars.has(url)) return mockAvatars.get(url) ?? null;
  let h = 0;
  for (let i = 0; i < url.length; i++) h = (h * 31 + url.charCodeAt(i)) >>> 0;
  const ready = url.startsWith(DEMO_PICTURE_BASE) || (!url.includes('/missing/') && h % 3 !== 0);
  const data = ready ? demoPicture(url, 256) : null;
  mockAvatars.set(url, data);
  return data;
}

function mockOwnSet(input: MessengerProfileInput): MessengerProfile {
  const markup = (input.about ?? '').trim();
  if ([...markup].length > BIO_MAX_CHARS) throw new Error('bio_too_long');
  const socials = (input.socials ?? []).map(demoSocialLink);
  const clean = (v: string | null | undefined) => v?.trim() || null;
  if (input.website && !/^https?:\/\/\S+$/.test(input.website.trim())) throw new Error('website must be a URL');
  const base = mockOwnProfile ?? emptyProfile(mockIdentity?.pubkey ?? 'ab'.repeat(32));
  mockOwnProfile = {
    ...base, name: clean(input.name), display_name: clean(input.display_name), about: demoStrip(markup) || null,
    website: clean(input.website), nip05: clean(input.nip05), lud16: clean(input.lud16),
    bio: demoBioParse(markup), bio_source: markup || null,
    socials: socials.filter((x, i) => socials.findIndex((y) => y.p === x.p && y.h === x.h) === i).slice(0, SOCIALS_MAX).map(demoSocialView),
    event_created_at: Math.floor(Date.now() / 1000),
  };
  return mockOwnProfile;
}

function mockAvatarSet(token: string, rect: CropRect): MessengerProfile {
  if (!mockAvatarPick || mockAvatarPick.token !== token) throw new Error('avatar_expired');
  const ok = [rect.x, rect.y, rect.w, rect.h].every(Number.isFinite) && rect.w > 0 && rect.h > 0
    && rect.x >= 0 && rect.y >= 0 && rect.x + rect.w <= 1.0001 && rect.y + rect.h <= 1.0001;
  if (!ok) throw new Error('avatar_bad_crop');
  if (!mockMediaServers.some((m) => m.enabled)) throw new Error('err.media_no_server');
  const sha = Array.from({ length: 64 }, () => Math.floor(Math.random() * 16).toString(16)).join('');
  const url = `${DEMO_PICTURE_BASE}${sha}`;
  mockAvatars.set(url, demoPicture(url, 256));
  mockAvatarPick = null;
  mockOwnProfile = { ...(mockOwnProfile ?? emptyProfile(mockIdentity?.pubkey ?? 'ab'.repeat(32))), picture: url, event_created_at: Math.floor(Date.now() / 1000) };
  return mockOwnProfile;
}

function mockCardSend(chat: string, pubkey: string | null, includePhone: boolean): MessengerMessage {
  const c = mockChat(chat.replace(/^dm:/, ''));
  const me = mockIdentity?.pubkey ?? 'ab'.repeat(32);
  const whose = pubkey && pubkey !== me ? pubkey : null;
  let card: CardView;
  if (!whose) {
    const phone = includePhone ? mockOwnPrivate.phone : null;
    const g = c.kind === 'group' ? mockGroups.find((x) => x.chat_id === c.id) : undefined;
    if (phone && g && g.kind !== 'private') throw new Error('phone_public_group');
    card = demoCard(mockOwnProfile ?? emptyProfile(me), phone, { is_me: true });
  } else {
    const contact = mockContacts.find((x) => x.pubkey === whose);
    card = demoCard(contact?.profile ?? emptyProfile(whose), null, { is_contact: !!contact });
  }
  const now = Math.floor(Date.now() / 1000);
  const m: MessengerMessage = {
    id: `local:${Date.now().toString(16)}${Math.random().toString(16).slice(2, 8)}`, chat_id: c.id, direction: 'out', status: 'sent',
    content_type: 'contact', text: null, sender_pubkey: me, reply_to: null, created_at: now, edited_at: null, deleted: false,
    failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: null, card,
  };
  mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
  mockChats = mockChats.map((x) => (x.id === c.id ? { ...x, last_message_at: now, last_preview: `👤 ${card.label}` } : x));
  return m;
}

function mockCardAccept(messageId: string): MessengerMessage {
  const m = mockFind(messageId);
  const card = m?.card;
  if (!m || !card) throw new Error('card_unknown');
  if (card.is_me) throw new Error('card_is_me');
  if (!mockContacts.some((x) => x.pubkey === card.pubkey)) {
    const now = Math.floor(Date.now() / 1000);
    const profile: MessengerProfile = {
      ...emptyProfile(card.pubkey), npub: card.npub, name: card.name, display_name: card.display_name, website: card.website,
      bio: card.bio, socials: card.socials, about: card.bio.map((x) => (x.kind === 'break' ? '\n' : x.text)).join('') || null,
    };
    mockContacts = [{ pubkey: card.pubkey, npub: card.npub, nickname: null, note: null, followed: false, profile, created_at: now, updated_at: now }, ...mockContacts];
  }
  // A phone is kept only from its owner's own card.
  if (card.phone && card.pubkey === m.sender_pubkey) mockContactPhones[card.pubkey] = card.phone;
  m.card = { ...card, is_contact: true };
  return { ...m };
}

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

// ── File transfers of the browser preview ─────────────────────────────────
// The demo's transfers stand still in the states they show until a button
// moves them; a file sent in the preview plays its upload in about 8 s. The
// preview has no host to send events: the stores hear them by `onDemoEvent`.

type DemoListener = (ev: MessengerUiEvent) => void;
const demoListeners = new Set<DemoListener>();

/** The browser preview's runtime events (store.svelte.ts listens). */
export function onDemoEvent(fn: DemoListener): () => void {
  demoListeners.add(fn);
  return () => { demoListeners.delete(fn); };
}

function demoEmit(name: string, payload: unknown) {
  for (const fn of demoListeners) fn({ name, payload });
}

const MOCK_CHUNK = 4 * 1024 * 1024;
const mockTransfers = new Map<string, MessengerTransfer>((demo?.transfers ?? []).map((x) => [x.id, x]));
/** Ticks spent in the current stage. */
const mockTicks = new Map<string, number>();
/** Pictures sent compressed: they go through "preparing" first. */
const mockCompress = new Set<string>();
const mockRunning = new Set<string>();

function mockTransferPut(t: MessengerTransfer) {
  mockTransfers.set(t.id, t);
  demoEmit('transfer.progress', transferProgress(t));
}

function mockTransferOf(id: string): MessengerTransfer {
  const t = mockTransfers.get(id);
  if (!t) throw new Error('err.not_found');
  return t;
}

/** Changes a message of the demo (`null` removes it); the open chat reads it again. */
function mockMessageUpdate(id: string, change: (m: MessengerMessage) => MessengerMessage | null) {
  for (const [chat, list] of Object.entries(mockMessages)) {
    if (!list.some((m) => m.id === id)) continue;
    mockMessages[chat] = list.flatMap((m) => {
      if (m.id !== id) return [m];
      const next = change(m);
      return next ? [next] : [];
    });
    demoEmit('dm.updated', { chat_id: chat });
  }
}

/** One step of a running transfer: about 6 s of bytes, a moment for each stage around them. */
function mockStep(t: MessengerTransfer): MessengerTransfer {
  const ticks = (mockTicks.get(t.id) ?? 0) + 1;
  mockTicks.set(t.id, ticks);
  const to = (stage: TransferStage, over: Partial<MessengerTransfer> = {}): MessengerTransfer => {
    mockTicks.set(t.id, 0);
    return { ...t, stage, ...over };
  };
  const total = Math.max(1, Math.ceil(t.size / t.chunk_size));
  const stored = Math.floor(t.done_bytes / t.chunk_size);
  // A photo being made smaller is still queued, as the runtime tells it.
  switch (t.status === 'queued' && t.stage !== 'preparing' ? 'queued' : t.stage) {
    case 'queued': {
      const first = t.direction === 'down' ? 'downloading' : mockCompress.has(t.id) ? 'preparing' : t.done_bytes > 0 ? 'checking' : 'uploading';
      return to(first, { status: first === 'preparing' ? 'queued' : 'running', chunks_done: first === 'downloading' ? stored : 0, chunks_total: total });
    }
    case 'preparing':
      return ticks < 4 ? t : to('uploading', { status: 'running', chunks_done: 0 });
    case 'checking': {
      const done = Math.min(stored, t.chunks_done + Math.max(1, Math.ceil(stored / 4)));
      return done >= stored ? to('uploading', { chunks_done: stored }) : { ...t, chunks_done: done };
    }
    case 'uploading':
    case 'downloading': {
      const step = t.size / 24;
      const done = Math.min(t.size, t.done_bytes + step);
      const moved = { done_bytes: done, chunks_done: done >= t.size ? total : Math.floor(done / t.chunk_size), rate_bps: Math.round(step * 4), eta_secs: Math.round((t.size - done) / (step * 4)) };
      if (done < t.size) return { ...t, ...moved };
      const still = { ...moved, rate_bps: 0, eta_secs: null };
      return t.direction === 'up' ? to('publishing', still) : to('assembling', { ...still, chunks_done: 0 });
    }
    case 'assembling': {
      const done = Math.min(total, t.chunks_done + Math.max(1, Math.ceil(total / 6)));
      return done >= total ? to('verifying', { chunks_done: total }) : { ...t, chunks_done: done };
    }
    case 'publishing':
    case 'verifying':
      return ticks < 4 ? t : { ...t, status: 'done', local_path: t.local_path ?? '/dev/mock' };
  }
  return t;
}

/** Moves a transfer four times a second until it stops or ends. */
function mockRun(id: string) {
  if (mockRunning.has(id)) return;
  mockRunning.add(id);
  const tick = () => {
    const t = mockTransfers.get(id);
    if (!t || (t.status !== 'running' && t.status !== 'queued')) { mockRunning.delete(id); return; }
    const next = mockStep(t);
    mockTransferPut(next);
    if (next.status !== 'done') { setTimeout(tick, 250); return; }
    mockRunning.delete(id);
    if (!next.message_id) return;
    if (next.direction === 'up') mockMessageUpdate(next.message_id, (m) => ({ ...m, status: 'sent', failure_reason: null }));
    else mockMessageUpdate(next.message_id, (m) => ({ ...m, media: { ...m.media, local_path: next.local_path } }));
  };
  setTimeout(tick, 250);
}

function mockResume(t: MessengerTransfer) {
  const stage: TransferStage = t.direction === 'down' ? 'downloading' : t.done_bytes > 0 ? 'checking' : t.stage === 'preparing' ? 'preparing' : 'uploading';
  mockTicks.set(t.id, 0);
  mockTransferPut({ ...t, status: stage === 'preparing' ? 'queued' : 'running', stage, chunks_done: stage === 'checking' ? 0 : t.chunks_done, failure_reason: null, retry_at_ms: null });
  mockRun(t.id);
}

/** A file picked in the preview: what its name says it is. */
function mockKind(name: string): { kind: MediaKind; mime: string } {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  if (['jpg', 'jpeg', 'png', 'gif', 'webp'].includes(ext)) return { kind: 'image', mime: ext === 'png' ? 'image/png' : 'image/jpeg' };
  if (['mp4', 'webm', 'mov'].includes(ext)) return { kind: 'video', mime: 'video/mp4' };
  if (['mp3', 'ogg', 'wav'].includes(ext)) return { kind: 'audio', mime: 'audio/mpeg' };
  return { kind: 'file', mime: ext === 'pdf' ? 'application/pdf' : 'application/octet-stream' };
}

const MOCK_EMOJI_TOP = ['👍', '😂', '🔥', '🙏', '❤️', '🎉', '🤔', '👀'];

/** Browser preview only: the runtime keeps reactions in the app. Toggles mine. */
function mockReact(id: string, emoji: string): MessengerMessage {
  const m = mockFind(id);
  if (!m || m.deleted) throw new Error('dm_message_unknown');
  if (!emoji || emoji.length > 32 || /[\x00-\x7f]|\s/.test(emoji)) throw new Error('reaction_invalid');
  const list = (m.reactions ?? []).map((r) => ({ ...r }));
  const had = list.find((r) => r.emoji === emoji);
  if (had?.mine) {
    had.mine = false;
    had.count -= 1;
  } else {
    const mine = list.filter((r) => r.mine).length;
    if (mine >= 3 || (!had && list.length >= 3)) throw new Error('reaction_limit');
    if (had) { had.mine = true; had.count += 1; } else list.push({ emoji, count: 1, mine: true });
  }
  m.reactions = list.filter((r) => r.count > 0);
  return { ...m };
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

/** The demo's first two approved contacts: one online, one seen an hour ago; nobody while the switch is off. */
function mockPresence(): MessengerPresence[] {
  if (!mockPrivacy.presence || !demo) return [];
  const now = Math.floor(Date.now() / 1000);
  const [online, away] = demo.chats.filter((c) => c.kind === 'dm' && c.mode === 'full_chat').map((c) => c.peer_pubkey ?? '');
  const list: MessengerPresence[] = [];
  if (online) list.push({ peer: online, seen_at: now - 10, online_until: now + 70 });
  if (away) list.push({ peer: away, seen_at: now - 3600, online_until: now - 3520 });
  return list;
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
      link: 'ok',
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
    const m: MessengerMessage = { id: `${Date.now().toString(16)}${Math.random().toString(16).slice(2)}`, chat_id: c.id, direction: 'out', status: 'sent', content_type: 'text', text: String(a?.text), sender_pubkey: 'ab'.repeat(32), reply_to: target ? { id: target.id, sender_pubkey: target.sender_pubkey, text: target.text } : null, created_at: now, edited_at: null, deleted: false, failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: null };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    mockChats = mockChats.map((x) => x.id === c.id ? { ...x, last_message_at: now, last_preview: m.text } : x);
    return m;
  },
  messenger_dm_react: (a) => mockReact(String(a?.messageId), String(a?.emoji)),
  messenger_emoji_used: () => undefined,
  messenger_emoji_top: (a) => MOCK_EMOJI_TOP.slice(0, Number(a?.n ?? 24)),
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
    const { kind, mime } = mockKind(name);
    const size = kind === 'image' ? 3_400_000 : 24 * 1024 * 1024;
    const id = `local:${Date.now().toString(16)}${Math.random().toString(16).slice(2, 8)}`;
    const transferId = `demo-send-${id.slice(6)}`;
    const m: MessengerMessage = { id, chat_id: c.id, direction: 'out', status: 'uploading', content_type: 'media', text: (a?.caption as string) ?? null, sender_pubkey: 'ab'.repeat(32), reply_to: null, created_at: now, edited_at: null, deleted: false, failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: { name, mime, size, kind, local_path: String(a?.path), transfer_id: transferId, ...(a?.batch ? { batch: String(a.batch) } : {}) } };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    mockChats = mockChats.map((x) => (x.id === c.id ? { ...x, last_message_at: now, last_preview: `📎 ${name}` } : x));
    // The upload plays: a picture is compressed first unless it goes as it is.
    const chunk = kind === 'image' ? 512 * 1024 : MOCK_CHUNK;
    if (kind === 'image' && !a?.original) mockCompress.add(transferId);
    const t: MessengerTransfer = {
      id: transferId, direction: 'up', message_id: id, chat_id: c.id, file_name: name, mime, size, status: 'queued', done_bytes: 0, attempts: 0,
      failure_reason: null, local_path: String(a?.path), stage: 'queued', chunks_done: 0, chunks_total: Math.ceil(size / chunk), chunk_size: chunk,
      rate_bps: 0, eta_secs: null, retry_at_ms: null,
    };
    setTimeout(() => { mockTransferPut(t); mockRun(transferId); }, 0);
    return m;
  },
  messenger_media_import: (a) => {
    const path = String(a?.path);
    const name = path.split(/[\\/]/).pop() ?? 'file';
    // `huge` in the name: a file over the limit, refused as the runtime refuses it.
    if (/huge/i.test(name)) throw new Error(`err.file_too_large: ${name}`);
    const { kind } = mockKind(name);
    const image = kind === 'image';
    return { path, name, kind, size: image ? 3_400_000 : 24 * 1024 * 1024, preview: image ? demoPicture(name, 160) : null } satisfies MessengerPicked;
  },
  messenger_media_grant_access: () => undefined,
  messenger_dm_send_recording: (a) => {
    const c = mockChat(String(a?.to));
    const r = (a?.recording ?? {}) as { kind: MediaKind; mime: string; duration_ms: number; waveform: number[] | null; data_base64: string };
    const now = Math.floor(Date.now() / 1000);
    const m: MessengerMessage = { id: `local:${Date.now().toString(16)}`, chat_id: c.id, direction: "out", status: "sent", content_type: "media", text: null, sender_pubkey: "ab".repeat(32), reply_to: null, created_at: now, edited_at: null, deleted: false, failure_reason: null, delivered_at: null, read_at: null, seen_by: [], reactions: [], media: { name: `${r.kind}.webm`, mime: r.mime, size: Math.round((r.data_base64.length * 3) / 4), kind: r.kind, duration_ms: r.duration_ms, waveform: r.waveform ?? undefined, local_path: "/dev/mock", mock_data: `data:${r.mime.split(";")[0]};base64,${r.data_base64}` } };
    mockMessages[c.id] = [...(mockMessages[c.id] ?? []), m];
    return m;
  },
  // Fetched only when asked: the demo's other files stay where they are.
  messenger_media_download: (a) => {
    if (!a?.manual) return null;
    const m = mockFind(String(a?.messageId));
    const md = m && mediaOf(m);
    if (!m || !md) return null;
    const known = [...mockTransfers.values()].reverse().find((t) => t.message_id === m.id && t.direction === 'down');
    if (known && known.status !== 'done' && known.status !== 'cancelled') {
      if (known.status === 'failed' || known.status === 'paused') mockResume(known);
      return null;
    }
    const t: MessengerTransfer = {
      id: `demo-fetch-${Date.now().toString(16)}`, direction: 'down', message_id: m.id, chat_id: m.chat_id, file_name: md.name, mime: md.mime,
      size: md.size, status: 'queued', done_bytes: 0, attempts: 0, failure_reason: null, local_path: null, stage: 'queued', chunks_done: 0,
      chunks_total: md.chunks ?? Math.max(1, Math.ceil(md.size / MOCK_CHUNK)), chunk_size: MOCK_CHUNK, rate_bps: 0, eta_secs: null, retry_at_ms: null,
    };
    mockTransferPut(t);
    mockRun(t.id);
    return null;
  },
  messenger_media_transfer: (a) => [...mockTransfers.values()].reverse().find((t) => t.message_id === a?.messageId) ?? null,
  messenger_media_transfers: () => [...mockTransfers.values()].filter((t) => t.status !== 'done' && t.status !== 'cancelled').reverse(),
  messenger_media_retry_failed: () => {
    const failed = [...mockTransfers.values()].filter((t) => t.status === 'failed' || t.status === 'waiting_retry');
    failed.forEach(mockResume);
    return failed.length;
  },
  messenger_media_pause: (a) => {
    const t = mockTransferOf(String(a?.transferId));
    mockTransferPut({ ...t, status: 'paused', rate_bps: 0, eta_secs: null, retry_at_ms: null });
  },
  // Also "now" for one waiting for its next attempt, and "again" for a failed one.
  messenger_media_resume: (a) => mockResume(mockTransferOf(String(a?.transferId))),
  messenger_media_cancel: (a) => {
    const t = mockTransferOf(String(a?.transferId));
    mockTransferPut({ ...t, status: 'cancelled', rate_bps: 0, eta_secs: null, retry_at_ms: null });
    // A cancelled upload takes its message with it.
    if (t.direction === 'up' && t.message_id) mockMessageUpdate(t.message_id, () => null);
  },
  messenger_media_save_as: () => undefined,
  messenger_media_data_url: (a) => {
    const md = mockFind(String(a?.messageId))?.media;
    // A picture of the demo that is "here" gets one drawn for it.
    return (md?.mock_data as string | undefined) ?? (md?.kind === 'image' && md.local_path ? demoPicture(String(md.name), 640, 480) : null);
  },
  messenger_media_local_path: () => null,
  messenger_media_url: () => null,
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
  messenger_profile_own_set: (a) => mockOwnSet((a?.input ?? {}) as MessengerProfileInput),
  messenger_nip05_verify: () => false,
  messenger_social_platforms: (): SocialPlatform[] => DEMO_PLATFORMS,
  messenger_bio_parse: (a): Span[] => demoBioParse(String(a?.markup ?? '')),
  messenger_avatar_prepare: (a): AvatarPreview => {
    const source = String(a?.source ?? '');
    if (!/\.(jpe?g|png|webp|gif|bmp)$/i.test(source) && !source.startsWith('content://')) throw new Error('avatar_unsupported');
    const preview = demoPicture(source, 800, 600);
    if (!preview) throw new Error('avatar_corrupt');
    mockAvatarPick = { token: `mock-${Date.now().toString(16)}`, preview };
    return { token: mockAvatarPick.token, preview, width: 1600, height: 1200 };
  },
  messenger_avatar_set: (a) => mockAvatarSet(String(a?.token), (a?.rect ?? {}) as CropRect),
  messenger_avatar_remove: () => {
    mockOwnProfile = { ...(mockOwnProfile ?? emptyProfile(mockIdentity?.pubkey ?? 'ab'.repeat(32))), picture: null, event_created_at: Math.floor(Date.now() / 1000) };
    return mockOwnProfile;
  },
  messenger_avatar_cached: (a) => mockAvatarCached(String(a?.url)),
  messenger_own_private_get: (): OwnPrivateView => mockOwnPrivate,
  messenger_own_private_set: (a): OwnPrivateView => {
    mockOwnPrivate = { phone: demoPhone(String(a?.phone ?? '')), share_phone: Boolean(a?.sharePhone) };
    return mockOwnPrivate;
  },
  messenger_contact_private_get: (a): ContactPrivateView => ({ pubkey: String(a?.pubkey), phone: mockContactPhones[String(a?.pubkey)] ?? null }),
  messenger_card_send: (a) => mockCardSend(String(a?.chat), (a?.pubkey as string | null) ?? null, Boolean(a?.includePhone)),
  messenger_card_accept: (a) => mockCardAccept(String(a?.messageId)),
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
  messenger_privacy_get: () => mockPrivacy,
  messenger_presence_list: () => mockPresence(),
  messenger_presence_foreground: () => undefined,
  messenger_privacy_set: (a) => { mockPrivacy = { read_receipts: Boolean(a?.readReceipts), presence: Boolean(a?.presence) }; return mockPrivacy; },
  messenger_desktop_notify_get: () => mockDesktopNotify,
  messenger_desktop_notify_set: (a) => { mockDesktopNotify = { ...mockDesktopNotify, enabled: Boolean(a?.enabled), sound: Boolean(a?.sound) }; return mockDesktopNotify; },
  messenger_desktop_notify_test: () => undefined,
  messenger_notice_words: () => undefined,
  messenger_desktop_notify_keep_running: () => { mockDesktopNotify = { ...mockDesktopNotify, close_to_tray: true }; return mockDesktopNotify; },
  messenger_notify_set: (a) => { mockNotify = { ...mockNotify, content: a?.content as MessengerNotifySettings['content'], lockscreen_hidden: Boolean(a?.lockscreenHidden) }; return mockNotify; },
};

// Calls in the browser preview: played by calls/demo.ts (`messenger.demo.call`).
Object.assign(devMocks, demoCallMocks({
  demo: !!demo,
  emit: demoEmit,
  chat: (peer) => mockChat(peer),
  lines: (chatId) => (mockMessages[chatId] ??= []),
  touch: (chatId, at) => { mockChats = mockChats.map((c) => (c.id === chatId ? { ...c, last_message_at: at, last_preview: '📞' } : c)); },
  me: () => mockIdentity?.pubkey ?? '',
  members: (groupId) => mockGroups.find((g) => g.id === groupId && g.membership === 'joined')?.members.map((m) => m.pubkey) ?? [],
}));

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

  /** What the other side learns: read receipts and presence. */
  privacy: {
    get: () => invoke<MessengerPrivacy>('messenger_privacy_get'),
    set: (readReceipts: boolean, presence: boolean) =>
      invoke<MessengerPrivacy>('messenger_privacy_set', { readReceipts, presence }),
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
    /** Toggles my `emoji` on a message: puts it, or takes it back when it is already mine. */
    react: (messageId: string, emoji: string) => invoke<MessengerMessage>('messenger_dm_react', { messageId, emoji }),
    delete: (messageId: string, forEveryone: boolean) => invoke<void>('messenger_dm_delete', { messageId, forEveryone }),
    retry: (messageId: string) => invoke<void>('messenger_dm_retry', { messageId }),
    relation: (peer: string) => invoke<MessengerRelation>('messenger_dm_relation', { peer }),
    action: (peer: string, action: DmAction) => invoke<MessengerRelation>('messenger_dm_action', { peer, action }),
    blocked: () => invoke<string[]>('messenger_dm_blocked'),
  },


  /** When approved contacts are online; empty while the presence switch is off. */
  presence: {
    list: () => invoke<MessengerPresence[]>('messenger_presence_list'),
    /** `true` is a lease of 90 s: say it again every 45 s while the page is visible. */
    foreground: (visible: boolean) => invoke<void>('messenger_presence_foreground', { visible }),
  },

  /** Which emoji I use most, the same on every device of mine. */
  emoji: {
    /** I picked this emoji in the composer (a reaction counts by itself). */
    used: (emoji: string) => invoke<void>('messenger_emoji_used', { emoji }),
    /** Most used first. */
    top: (n: number) => invoke<string[]>('messenger_emoji_top', { n }),
  },

  /**
   * Calls of two. What a call does comes back as runtime events:
   * `call.incoming`, `call.state`, `call.ended`, `call.stats`, `call.level`.
   * Refusals are the runtime's sentences (`calls/words.ts` words them).
   */
  calls: {
    /** `peer`: hex or npub; only a contact in a mutual chat. */
    start: (peer: string, media: CallMedia = 'audio') => invoke<CallView>('messenger_call_start', { peer, media }),
    accept: (callId: string) => invoke<CallView>('messenger_call_accept', { callId }),
    /** My other devices stop ringing too. */
    decline: (callId: string) => invoke<void>('messenger_call_decline', { callId }),
    /** Hangs up, or gives up calling. */
    end: (callId: string) => invoke<void>('messenger_call_end', { callId }),
    mute: (muted: boolean) => invoke<CallView>('messenger_call_mute', { muted }),
    /** For the next call; the one under way keeps its way. */
    setPolicy: (policy: RelayPolicy) => invoke<CallStateView>('messenger_call_set_policy', { policy }),
    /** My own nodes, replacing the list (`address:port#id`, a key for a private one). */
    setNodes: (nodes: CallNodeInput[]) => invoke<CallStateView>('messenger_call_set_nodes', { nodes }),
    /** The call under way, the policy, the nodes, whether this build can call, whether calls ring here. */
    state: () => invoke<CallStateView>('messenger_call_get_state'),
    /**
     * Calls ring on this device, or not: off, a call does not ring or show
     * here; my other devices still ring.
     */
    setIncoming: (enabled: boolean) => invoke<CallStateView>('messenger_call_set_incoming', { enabled }),
    /**
     * A phone: where the sound of the call goes. `list` answers the routes
     * there are and the one in use; `set` sends the sound to `route`. A
     * change made elsewhere (a headset plugged in) is the event
     * `call.audio_route` with the same answer. Refused on a computer and
     * while no call holds the sound.
     */
    audioRoute: (op: 'list' | 'set', route?: CallAudioRoute) =>
      invoke<CallAudioRoutes>('messenger_call_audio_route', { input: { op, route: route ?? null } }),
    /**
     * My video in the call under way: a camera, a screen, or off. The peer
     * is told; a camera that will not open is refused and the call goes on
     * without video.
     */
    setVideo: (input: VideoInput) => invoke<CallView>('messenger_call_set_video', { input }),
    /** The next camera (or `camera`): at once when mine is on, for the next time otherwise. */
    switchCamera: (camera?: string) => invoke<CallView>('messenger_call_switch_camera', { camera: camera ?? null }),
    /** A computer's cameras, the default first; empty on a phone (`front`/`back`). */
    cameras: () => invoke<CameraInfo[]>('messenger_call_list_cameras'),
    /** A computer: the screens and windows that can be shown. */
    screens: () => invoke<ScreenInfo[]>('messenger_call_list_screens'),
    /** A computer: my screen (or the window `screen`) instead of my camera. */
    shareScreen: (screen?: string) => invoke<CallView>('messenger_call_share_screen', { screen: screen ?? null }),
    /** How big my video goes, from the next time it goes on. */
    setVideoQuality: (quality: VideoQuality) => invoke<CallStateView>('messenger_call_set_video_quality', { quality }),
    /**
     * The frames of `track` of the call under way: `onframe` gets each
     * message of the channel as it came (calls/video.ts reads them) until
     * the last one or `videoUnsubscribe`. Answers the subscription's id.
     * Every message but the last is acknowledged with `videoAck`: the next
     * one comes only after that.
     */
    videoSubscribe: async (track: VideoTrack, onframe: (data: ArrayBuffer) => void): Promise<number> => {
      if (isTauri) {
        const { Channel, invoke: call } = await import('@tauri-apps/api/core');
        const channel = new Channel<ArrayBuffer>();
        channel.onmessage = onframe;
        return call<number>('messenger_call_video_subscribe', { track, channel });
      }
      // The preview's channel: calls/demo.ts sends its test pictures to it.
      return invoke<number>('messenger_call_video_subscribe', { track, channel: { onmessage: onframe } });
    },
    /**
     * The page took the frame `seq` of the subscription `id` (drew it or
     * let it go): the runtime sends the next one only after this.
     */
    videoAck: (id: number, seq: number) => invoke<void>('messenger_call_video_ack', { id, seq }),
    videoUnsubscribe: (id: number) => invoke<void>('messenger_call_video_unsubscribe', { id }),
  },

  /**
   * Calls of a group: a room on a call node, the group told by its own
   * notes. One call of either kind at a time: the runtime refuses a group
   * call during a call of two and the other way round. Events:
   * `group_call.state`, `group_call.started` (the banner of a group, at
   * every change of who is in), `group_call.ended`, `group_call.level`.
   */
  groupCalls: {
    /** A room on a node, me in it, the group told. Refused while a call is on in the group already: join it. */
    start: (groupId: string, media: CallMedia = 'audio') => invoke<GroupCallView>('messenger_group_call_start', { groupId, media }),
    /** Into the call that is on in the group (the banner's button). */
    join: (groupId: string) => invoke<GroupCallView>('messenger_group_call_join', { groupId }),
    /** Out of the room; the last one out ends the call for the group. */
    leave: () => invoke<void>('messenger_group_call_leave'),
    mute: (muted: boolean) => invoke<GroupCallView>('messenger_group_call_mute', { muted }),
    /** My video in the room: a camera, a screen, or off. */
    setVideo: (input: VideoInput) => invoke<GroupCallView>('messenger_group_call_set_video', { input }),
    /** The next camera (or `camera`): at once when mine is on, for the next time otherwise; `front`/`back` on a phone. */
    switchCamera: (camera?: string) => invoke<GroupCallView>('messenger_group_call_switch_camera', { camera: camera ?? null }),
    /**
     * The layer of the video of the seat `participant` I want, for the
     * size of its tile: `q` (a quarter), `h` (a half), `f` (the full
     * size). Refused by a node without simulcast.
     */
    setLayer: (participant: number, rid: string) => invoke<void>('messenger_group_call_set_layer', { participant, rid }),
    /** The room I am in, and the call announced in `groupId` when one is. */
    state: (groupId?: string) => invoke<GroupCallState>('messenger_group_call_get_state', { groupId: groupId ?? null }),
    /**
     * The frames of the video of one seat, by the m-line of its track
     * (`GroupParticipant.video_mid`), as `calls.videoSubscribe` carries
     * them: acknowledged and taken back with `calls.videoAck` and
     * `calls.videoUnsubscribe`.
     */
    videoSubscribe: async (mid: string, onframe: (data: ArrayBuffer) => void): Promise<number> => {
      if (isTauri) {
        const { Channel, invoke: call } = await import('@tauri-apps/api/core');
        const channel = new Channel<ArrayBuffer>();
        channel.onmessage = onframe;
        return call<number>('messenger_group_call_video_subscribe', { mid, channel });
      }
      return invoke<number>('messenger_group_call_video_subscribe', { mid, channel: { onmessage: onframe } });
    },
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
    /**
     * A picked file as the composer holds it (Android copies `content://` in first).
     * A file over `MAX_SEND_BYTES` is refused with `err.file_too_large`.
     */
    importPicked: (path: string) => invoke<MessengerPicked>('messenger_media_import', { path }),
    /** `batch`: the same for files picked together. `original`: a picture goes as it is, not compressed. */
    /** `poster`: a frame of a video, the other side's preview before it fetches the file. */
    sendFile: (to: string, path: string, caption?: string, batch?: string, original = false, poster: MessengerPoster | null = null) =>
      invoke<MessengerMessage>('messenger_dm_send_file', { to, path, caption: caption?.trim() || null, batch: batch ?? null, original, poster }),
    download: (messageId: string, manual: boolean) => invoke<string | null>('messenger_media_download', { messageId, manual }),
    transfer: (messageId: string) => invoke<MessengerTransfer | null>('messenger_media_transfer', { messageId }),
    /** Every transfer queued, running, waiting, paused or failed, newest first. */
    transfers: () => invoke<MessengerTransfer[]>('messenger_media_transfers'),
    /** Starts again every failed transfer, every one the closing of the app paused and every one waiting for its next attempt (never a pause the user made); how many. */
    retryFailed: () => invoke<number>('messenger_media_retry_failed'),
    pause: (transferId: string) => invoke<void>('messenger_media_pause', { transferId }),
    resume: (transferId: string) => invoke<void>('messenger_media_resume', { transferId }),
    cancel: (transferId: string) => invoke<void>('messenger_media_cancel', { transferId }),
    saveAs: (messageId: string, dest: string) => invoke<void>('messenger_media_save_as', { messageId, dest }),
    dataUrl: (messageId: string) => invoke<string | null>('messenger_media_data_url', { messageId }),
    localPath: (messageId: string) => invoke<string | null>('messenger_media_local_path', { messageId }),
    /** Where the webview reads the attachment on this device by itself (the app's server on the loopback); `null` when it is not here or not a picture, video or sound. */
    url: (messageId: string) => invoke<string | null>('messenger_media_url', { messageId }),
    /** Opens with the default application; runnable files are only revealed in their folder. */
    open: (messageId: string) => invoke<void>('messenger_media_open', { messageId }),
  },

  profiles: {
    get: (pubkey: string) => invoke<MessengerProfile | null>('messenger_profile_get', { pubkey }),
    request: (pubkey: string) => invoke<void>('messenger_profile_request', { pubkey }),
    ownGet: () => invoke<MessengerProfile | null>('messenger_profile_own_get'),
    /**
     * Publishes my kind 0. `about` is the bio with its marks; socials as
     * typed (checked here). The picture is not in it: see `avatar`.
     * Errors: `bio_too_long`, `social_unknown_platform`, `social_bad_handle`, `profile_too_large`.
     */
    ownSet: (input: MessengerProfileInput) => invoke<MessengerProfile>('messenger_profile_own_set', { input }),
    verifyNip05: (pubkey: string) => invoke<boolean>('messenger_nip05_verify', { pubkey }),
    /** The platforms a social link can be of, in the order to offer them. */
    platforms: () => invoke<SocialPlatform[]>('messenger_social_platforms'),
    /** A bio as it will show: the live preview of the editor. */
    bioParse: (markup: string) => invoke<Span[]>('messenger_bio_parse', { markup }),
  },

  /** My own picture: picked from a file, cropped in the UI, re-encoded and uploaded by the runtime. */
  avatar: {
    /**
     * `source`: a picked path, or `content://` on Android. The picture is
     * held under a token for ten minutes, until another is picked.
     * Errors: `avatar_too_large`, `avatar_unsupported`, `avatar_corrupt`.
     */
    prepare: (source: string) => invoke<AvatarPreview>('messenger_avatar_prepare', { source }),
    /**
     * Crops (fractions 0..1 of the preview), uploads to my media servers and
     * republishes my profile. Errors: `avatar_expired`, `avatar_bad_crop`, `err.media_no_server`.
     */
    set: (token: string, rect: CropRect) => invoke<MessengerProfile>('messenger_avatar_set', { token, rect }),
    remove: () => invoke<MessengerProfile>('messenger_avatar_remove'),
    /**
     * The picture at `url` (https) as a `data:` url, or `null` while it is
     * fetched: `avatar.ready {url}` follows. Use `avatarStore`, not this.
     */
    cached: (url: string) => invoke<string | null>('messenger_avatar_cached', { url }),
  },

  /** My phone: never published, synced between my devices, sent only in my own card. */
  ownPrivate: {
    get: () => invoke<OwnPrivateView>('messenger_own_private_get'),
    /** `phone` as typed; `null` or empty removes it. Error: `phone_invalid`. */
    set: (phone: string | null, sharePhone: boolean) =>
      invoke<OwnPrivateView>('messenger_own_private_set', { phone: phone?.trim() || null, sharePhone }),
  },

  /** What a contact told me privately: the phone of its own card. */
  contactPrivate: {
    get: (pubkey: string) => invoke<ContactPrivateView>('messenger_contact_private_get', { pubkey }),
  },

  /** Contact cards as messages. */
  cards: {
    /**
     * Sends a card to `chat` (a person, `dm:<hex>` or `group:<id>`): mine
     * when `pubkey` is `null` (with my phone when `includePhone`), else
     * that person's public profile, never a phone.
     * Errors: `phone_public_group`, `card_too_large`.
     */
    send: (chat: string, pubkey: string | null, includePhone: boolean) =>
      invoke<MessengerMessage>('messenger_card_send', { chat, pubkey, includePhone }),
    /** "Add contact" on a received card. Errors: `card_unknown`, `card_is_me`. */
    accept: (messageId: string) => invoke<MessengerMessage>('messenger_card_accept', { messageId }),
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
