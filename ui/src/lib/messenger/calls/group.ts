// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The room of a group call on the screen: the order of its tiles, which
// one is large, how the grid is cut, which layer of a video a tile asks
// for, what the line under the group's name says. Pure, so it is tested
// without a screen.

import type { GroupCallView, GroupParticipant } from '../generated/calls';
import { clock } from './words';

type Translate = (key: string, params?: Record<string, string>) => string;

/** How a group call ended for me, as the screen says it for a moment after. */
export type GroupOver = 'left' | 'ended' | 'failed';

/**
 * The tiles in their order: the others by seat (the earlier in the room
 * first), mine last. Seats whose word of identity was not checked yet go
 * after the people: they are shown, nameless, but not heard.
 */
export function seatsInOrder(participants: GroupParticipant[]): GroupParticipant[] {
  const rank = (p: GroupParticipant) => (p.me ? 2 : p.verified ? 0 : 1);
  return [...participants].sort((a, b) => rank(a) - rank(b) || a.id - b.id);
}

/**
 * The seat shown large, or `null` for a grid of equals. A seat picked by a
 * tap stays large while it is there. Otherwise the one who speaks (or last
 * spoke) when their video goes: a voice without a picture gains nothing
 * from a large tile, nor does a track that sends no pictures (`showing`:
 * the frames of the seat's video come). My own tile is never large: I see
 * myself small.
 */
export function focusOf(
  participants: GroupParticipant[],
  pinned: number | null,
  speaker: number | null,
  showing: (seat: number) => boolean = () => true,
): number | null {
  if (pinned != null && participants.some((p) => p.id === pinned)) return pinned;
  const video = (p: GroupParticipant) => !p.me && seatSendsVideo(p, { video_local: false }, showing);
  const withVideo = (id: number | null) => participants.find((p) => p.id === id && video(p));
  const talking = participants.find((p) => p.speaking && video(p));
  if (talking) return talking.id;
  return withVideo(speaker)?.id ?? null;
}

/**
 * The seat sends a picture now: my camera (or screen) goes, or the frames
 * of another's video come (`showing`, from its tile). Not the m-line of its
 * video alone: every seat of a room has one from the start, its track
 * disabled until the camera goes on, and kept while the camera is off.
 */
export function seatSendsVideo(
  p: GroupParticipant,
  call: Pick<GroupCallView, 'video_local'>,
  showing: (seat: number) => boolean,
): boolean {
  if (p.me) return call.video_local;
  return p.verified && !!p.video_mid && showing(p.id);
}

/**
 * The seat's microphone is off, as far as this side knows: mine from my
 * own state; another's only when the room says so (a `muted` of the seat,
 * once the core carries one). Not `!audio`: that is the m-line of its
 * sound, there for every seat whether its microphone is on or off.
 */
export function seatMuted(p: GroupParticipant, call: Pick<GroupCallView, 'muted'>): boolean {
  if (p.me) return call.muted;
  return p.verified && (p as GroupParticipant & { muted?: boolean }).muted === true;
}

/**
 * The words of the strip of a group's chat, and what a screen reader says
 * of it: which call is on; never the clock that runs beside them.
 */
export function bannerKey(mine: boolean, media: GroupCallView['media'] | undefined): string {
  return mine ? 'msg_gcall_banner_mine' : media === 'video' ? 'msg_gcall_banner_video' : 'msg_gcall_banner';
}

/**
 * The m-line the runtime gives my own video for
 * (`messenger_group_call_video_subscribe`; `messenger-calls::group::MY_VIDEO_MID`):
 * the frames of what I send, for the tile of "me". Not a mid of the node:
 * `@` is no token character of an SDP mid, so no seat ever has it.
 */
export const MY_VIDEO_MID = '@me';

/**
 * Whether my own tile shows me mirrored: a face is, as in a mirror (a
 * computer's camera, a phone's front one); what a phone's back camera
 * sees is the world, and a screen is a page: neither is (calls/video.ts
 * `mirrorsLocal` for a call of two). The others get it unmirrored.
 */
export function mirrorsMine(call: Pick<GroupCallView, 'camera'>, screen: boolean): boolean {
  return !screen && call.camera !== 'back';
}

/**
 * The columns and rows of a grid of `n` tiles in a box, the tiles as large
 * as they can be at the ratio `ratio` (width / height) with `gap` between.
 */
export function gridShape(n: number, width: number, height: number, ratio = 16 / 10, gap = 8): { cols: number; rows: number } {
  if (n <= 1 || width <= 0 || height <= 0) return { cols: 1, rows: Math.max(1, n) };
  let best = { cols: 1, rows: n, size: -1 };
  for (let cols = 1; cols <= n; cols++) {
    const rows = Math.ceil(n / cols);
    const w = (width - gap * (cols - 1)) / cols;
    const h = (height - gap * (rows - 1)) / rows;
    if (w <= 0 || h <= 0) continue;
    // The tile is the largest box of the ratio inside its cell.
    const tileW = Math.min(w, h * ratio);
    if (tileW > best.size + 0.5) best = { cols, rows, size: tileW };
  }
  return { cols: best.cols, rows: best.rows };
}

/** The layers of a sender's video (`q;h;f` of its simulcast): a quarter, a half, the full size of 1280×720. */
export type Layer = 'q' | 'h' | 'f';

/**
 * The layer a tile of `width`×`height` CSS pixels needs on a screen of
 * `dpr` device pixels each: the smallest whose picture is not smaller
 * than the tile (320×180, 640×360, 1280×720), so nothing is sent that is
 * not seen.
 */
export function layerFor(width: number, height: number, dpr = 1): Layer {
  // The picture fills the tile cropped: the side that fills decides.
  const need = Math.max(width, (height * 16) / 9) * Math.max(1, dpr);
  if (need <= 400) return 'q';
  if (need <= 800) return 'h';
  return 'f';
}

/** The address of a node without its port and id: `108.61.171.68:8443#…` → `108.61.171.68`. */
export function nodeHost(reference: string): string {
  const addr = reference.split('#')[0] ?? '';
  const v6 = /^\[([^\]]+)\]:\d+$/.exec(addr);
  if (v6) return v6[1];
  return addr.replace(/:\d+$/, '');
}

/**
 * The way to the room, for its chip: `via` is my node when I sit in the
 * room through it (the cascade: my nearest node forwards to the room's),
 * `null` when I am on the room's node itself. Hosts only, as `nodeHost`.
 */
export function roomPath(call: Pick<GroupCallView, 'node' | 'home'>): { via: string | null; home: string } | null {
  const node = call.node || '';
  const home = call.home || node;
  if (!home) return null;
  return { via: node && node !== home ? nodeHost(node) : null, home: nodeHost(home) };
}

/** The words of the chip of the way to the room: one node, or mine and the room's. */
export function roomPathText(call: Pick<GroupCallView, 'node' | 'home'>, tr: Translate): string {
  const p = roomPath(call);
  if (!p) return '';
  return p.via ? tr('msg_gcall_via_cascade', { via: p.via, home: p.home }) : tr('msg_gcall_via_node', { node: p.home });
}

/**
 * The words under the group's name while the room is not talking; `null`
 * while it is (the clock shows then). `moving`: the room is on its way to
 * another node (its node went), told apart from a way merely lost.
 */
export function groupPhaseKey(call: Pick<GroupCallView, 'phase'>, moving = false): string | null {
  if (moving && call.phase !== 'in_room' && call.phase !== 'left') return 'msg_gcall_phase_moving';
  switch (call.phase) {
    case 'starting': return 'msg_gcall_phase_starting';
    case 'joining': return 'msg_gcall_phase_joining';
    case 'reconnecting': return 'msg_call_phase_reconnecting';
    case 'left': return 'msg_gcall_over_left';
    default: return null;
  }
}

/** How the call ended for me, said for a moment after. */
export function overKey(how: GroupOver): string {
  switch (how) {
    case 'left': return 'msg_gcall_over_left';
    case 'failed': return 'msg_call_ended_failed';
    default: return 'msg_gcall_over_ended';
  }
}

/** How long the call has been on at `now` (unix seconds): from my entry into the room, the start of the room before that. */
export function groupElapsed(call: Pick<GroupCallView, 'joined_at' | 'started_at'>, now: number): number {
  return Math.max(0, now - (call.joined_at ?? call.started_at));
}

/** The phase, or the clock while I am in the room; for a call just over, how it ended. */
export function groupStatusText(call: GroupCallView | null, over: GroupOver | null, now: number, tr: Translate, moving = false): string {
  if (call) {
    const key = groupPhaseKey(call, moving);
    return key ? tr(key) : clock(groupElapsed(call, now));
  }
  return over ? tr(overKey(over)) : '';
}

/** The people in the room as the screen counts them: every seat, mine included. */
export function peopleIn(call: Pick<GroupCallView, 'participants'>): number {
  return call.participants.length;
}
