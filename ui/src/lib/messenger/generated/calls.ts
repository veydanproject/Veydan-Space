// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** What a call carries. */
export type CallMedia = "audio" | "video";

/** Who called whom. */
export type CallDirection = "in" | "out";

/** Where a call is: ringing on their side, ringing here, ICE looking for a way, talking, restoring a lost way while talking, over. */
export type CallPhase = "outgoing" | "incoming" | "connecting" | "active" | "reconnecting" | "ended";

/** Why a call is `reconnecting`: the way went, my network changed, or the peer lost the way. */
export type ReconnectReason = "connection_lost" | "network_changed" | "peer_lost";

/** How the media goes: directly between the two, or through a relay. */
export type CallVia = "direct" | "relay";

/** How a call ended; `answered_elsewhere` means another device of mine took it. */
export type CallOutcome = "missed" | "declined" | "busy" | "ended" | "failed" | "answered_elsewhere";

/** Which way a call may take: `auto` (directly when it works), `relay_only` (my address stays behind a relay). */
export type RelayPolicy = "auto" | "relay_only";

/** What the nearest node allows, as it said. */
export type CallLimits = { turn_lifetime_secs: number, turn_kbps_per_allocation: number, credentials_ttl_secs: number, };

/** Which video of a call: mine as the camera sees it, or the peer's. */
export type VideoTrack = "local" | "remote";

/** What I send as video, as `messenger_call_set_video` takes it: nothing, a camera (by id, or the default; `front`/`back` on a phone), a screen or window (by id, or the first screen; a computer only). */
export type VideoInput = { "kind": "off" } | { "kind": "camera", id?: string, } | { "kind": "screen", id?: string, };

/** How big my video is sent, for the next time it goes on: 640×360 or 1280×720 at 30 fps. */
export type VideoQuality = "360p" | "720p";

/** The size of the frames of one video as they show. */
export type VideoSize = { width: number, height: number, };

/** A camera the engine can open, as `messenger_call_list_cameras` lists them; `id` is what `VideoInput` and `messenger_call_switch_camera` take. */
export type CameraInfo = { 
/**
 * What `VideoInput::Camera` and `messenger_call_switch_camera` take:
 * a device path, an index, whatever the platform names a camera by.
 */
id: string, name: string, };

/** A screen or a window that can be shared, as `messenger_call_list_screens` lists them. */
export type ScreenInfo = { 
/**
 * What `VideoInput::Screen` takes.
 */
id: string, title: string, 
/**
 * A window rather than a whole screen.
 */
window: boolean, };

/** The call as the screen shows it: the `call` of every `call.*` event and the answer of the call commands. */
export type CallView = { call_id: string, 
/**
 * The peer, hex.
 */
peer: string, chat_id: string, direction: CallDirection, media: CallMedia, phase: CallPhase, 
/**
 * How the media goes, once ICE settled.
 */
via?: CallVia, 
/**
 * Why the call is `reconnecting`; absent in every other phase.
 */
reconnect_reason?: ReconnectReason, muted: boolean, 
/**
 * When the invitation was made, unix seconds.
 */
started_at: number, 
/**
 * When it was taken.
 */
answered_at?: number, 
/**
 * The ids of the nodes this side uses, nearest first; empty when the
 * call goes with host candidates alone.
 */
nodes: Array<string>, limits?: CallLimits, 
/**
 * My video goes: the camera, or the screen (`video_screen`).
 */
video_local: boolean, 
/**
 * What I send is my screen, not my camera.
 */
video_screen: boolean, 
/**
 * The camera in use (or the one for the next time), by the id the
 * engine lists; absent for its default. On a phone `front` or `back`.
 */
camera?: string, 
/**
 * The peer's video goes, by its word (`call.video`) or by its
 * invitation; the screen shows a placeholder until frames come.
 */
video_remote: boolean, 
/**
 * The size of the frames each way, once some came; absent while that
 * video is off.
 */
video_local_size?: VideoSize, video_remote_size?: VideoSize, };

/** The payload of the runtime event `call.incoming`; `busy_with_group` says I sit in the room of a group call as it rings: its `accept` is refused, the screen shows "busy" instead of "answer". */
export type CallIncoming = { call: CallView, busy_with_group: boolean, };

/** The payload of the runtime event `call.ended`. */
export type CallEnded = { call: CallView, outcome: CallOutcome, 
/**
 * How long the call was answered, seconds; `null` for one that never was.
 */
duration_secs: number | null, };

/** The `stats` of the runtime event `call.stats`, as the engine tells them. */
export type CallStats = { rtt_ms?: number, bytes_sent?: number, bytes_received?: number, packets_lost?: number, jitter_ms?: number, };

/** A call node the client may use, as the settings list it. */
export type CallNodeView = { 
/**
 * `address:port#id`.
 */
reference: string, 
/**
 * The node's id, hex: what the TLS of its control channel is pinned to.
 */
id: string, 
/**
 * Whose it is: `own` (the setting), `project` (the manifest).
 */
class: string, 
/**
 * A key of a private node is kept for it.
 */
has_key: boolean, };

/** One own node, as `messenger_call_set_nodes` takes it. */
export type CallNodeInput = { 
/**
 * `address:port#id`.
 */
reference: string, 
/**
 * The access key of a private node.
 */
key?: string, };

/** The call under way, the policy and the nodes, as `messenger_call_get_state` answers. */
export type CallState = { 
/**
 * The call under way, if any.
 */
call: CallView | null, policy: RelayPolicy, 
/**
 * Every node a call may use, the most preferred first.
 */
nodes: Array<CallNodeView>, 
/**
 * This build can make a call (it has a media engine).
 */
available: boolean, 
/**
 * How big my video is sent, for the next time it goes on.
 */
video_quality: VideoQuality, 
/**
 * This device takes calls (on by default). Off: every invitation is
 * ignored here without a word, so that my other devices ring for it;
 * nothing goes on record on this device.
 */
incoming_enabled: boolean, };

/** Where I am with the room of a group call: making it, joining it, in it, restoring the way to the node, out of it. */
export type GroupCallPhase = "starting" | "joining" | "in_room" | "reconnecting" | "left";

/** One seat of the room of a group call; only a verified seat is a person and is heard. `video_mid` is what `messenger_group_call_video_subscribe` takes. */
export type GroupParticipant = { 
/**
 * The seat: the node's participant id.
 */
id: number, 
/**
 * Who sits there, hex, once its word of identity was checked.
 */
npub?: string, 
/**
 * The word of identity checked: a member, signed by its key, on
 * this seat. Only a verified seat is shown as a person and heard.
 */
verified: boolean, speaking: boolean, 
/**
 * The seat sends sound.
 */
audio: boolean, 
/**
 * The m-line of the seat's sound when it sends one.
 */
audio_mid?: string, 
/**
 * The m-line of the seat's video when it sends one: what
 * `messenger_group_call_video_subscribe` takes.
 */
video_mid?: string, me: boolean, };

/** The room of a group call I am in: the `call` of `group_call.state` and the answer of the group call commands. */
export type GroupCallView = { call_id: string, group_id: string, chat_id: string, phase: GroupCallPhase, media: CallMedia, muted: boolean, video_local: boolean, 
/**
 * The camera in use (or the one for the next time), by the id the
 * engine lists; absent for its default. On a phone `front` or `back`.
 */
camera?: string, 
/**
 * Who made the room, hex.
 */
started_by: string, 
/**
 * When the room was made, unix seconds.
 */
started_at: number, 
/**
 * When I got into the room.
 */
joined_at?: number, 
/**
 * The node the room is on, `address:port#id`.
 */
node: string, 
/**
 * My seat, once the node gave it.
 */
participant?: number, 
/**
 * The epoch of the keys I send with.
 */
epoch: number, 
/**
 * Every seat of the room, mine included, by seat.
 */
participants: Array<GroupParticipant>, limits?: CallLimits, 
/**
 * The most the room takes from one participant, kbit/s (0: no limit).
 */
kbps_per_participant: number, 
/**
 * Seats the room has at most (0: the node did not say).
 */
max_participants: number, };

/** A call announced in a group, for the banner of its chat: the `call` of `group_call.started` and `group_call.ended`. */
export type GroupCallAnnounced = { call_id: string, group_id: string, chat_id: string, media: CallMedia, 
/**
 * Who made the room, hex.
 */
started_by: string, started_at: number, 
/**
 * The members in the room by their own word, hex.
 */
participants: Array<string>, 
/**
 * I am in this room.
 */
joined: boolean, };

/** The payload of the runtime event `group_call.ended`. */
export type GroupCallEnded = { call: GroupCallAnnounced, outcome: CallOutcome, 
/**
 * From the start of the room to the end, seconds; `null` when unknown.
 */
duration_secs: number | null, };

/** The payload of the runtime event `group_call.level`: how loud one seat is. */
export type GroupCallLevel = { call_id: string, participant: number, level: number, };

/** The room I am in and the call announced in a group, as `messenger_group_call_get_state` answers. */
export type GroupCallState = { call: GroupCallView | null, announced: GroupCallAnnounced | null, };
