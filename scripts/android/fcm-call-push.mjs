#!/usr/bin/env node
// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Sends the push of a call to one phone by hand, the way VPush would
// (services/push/spec/protocol.md, "What arrives on the device" and the
// tag `call`): a `dm` push with the gift wrap inside, priority high, a
// `ttl` of a minute, the collapse key `call`. For the check of the call
// plugin on a phone whose app is in the background or dead, without a
// deployed push server (the plan of calls, stage 5).
//
//   node scripts/android/fcm-call-push.mjs \
//     --account <the service account of Firebase>.json \
//     --token <the FCM token of the phone> \
//     --to <hex pubkey of the phone> [--relay wss://eu-1-relay-1.veydan.net --key <api key>]
//     [--event <file with the wrap as JSON>] [--since <secs>] [--type dm]
//
// The wrap comes from `--event`, or is the newest kind-1059 for `--to`
// on the relay (the invitation `messenger-cli call <phone>` just sent;
// the relay of the project needs its api key, from the embedded manifest
// crates/messenger/transport/manifest/embedded.json). The token: with
// `adb shell setprop log.tag.VeydanPush VERBOSE` the app writes
// `token=…` to logcat when it asks for it (the push plugin, getToken).
//
// Nothing of this is a secret the repository keeps: the service account
// is kept out of git, the token on the phone.

import { readFileSync } from "node:fs";
import { createSign } from "node:crypto";

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const account = flag("--account");
const token = flag("--token");
const to = flag("--to");
const relay = flag("--relay", "wss://eu-1-relay-1.veydan.net");
const key = flag("--key");
const eventFile = flag("--event");
// The outside of a wrap carries a random time up to two days back (NIP-59),
// so the window must reach that far for the invitation sent a moment ago.
const since = Number(flag("--since", String(2 * 86400 + 600)));
const type = flag("--type", "dm");
// `--dry`: the wrap from the relay to stdout, nothing sent (no account needed).
const dry = args.includes("--dry");
if ((!dry && (!account || !token)) || (!to && !eventFile)) {
  console.error("usage: fcm-call-push.mjs --account <sa.json> --token <fcm token> (--to <hex|npub> [--relay <wss> --key <api key>] | --event <file>) [--dry]");
  process.exit(2);
}

/** A hex pubkey as it is; an npub decoded (bech32, no checksum verified: the relay answers nothing for a wrong one). */
function hexOf(key) {
  if (/^[0-9a-f]{64}$/.test(key)) return key;
  if (!key.startsWith("npub1")) throw new Error(`not a pubkey: ${key}`);
  const alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
  const words = [...key.slice(5, -6)].map((c) => alphabet.indexOf(c));
  let bits = 0, acc = 0;
  const out = [];
  for (const w of words) {
    acc = (acc << 5) | w;
    bits += 5;
    if (bits >= 8) { bits -= 8; out.push((acc >> bits) & 0xff); }
  }
  return Buffer.from(out).toString("hex");
}

/** The newest kind-1059 wrap for `to` on the relay, within `since` seconds. */
async function newestWrap() {
  const url = key ? `${relay}/?key=${key}` : relay;
  const ws = new WebSocket(url);
  const filter = { kinds: [1059], "#p": [hexOf(to)], since: Math.floor(Date.now() / 1000) - since, limit: 500 };
  const wraps = [];
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the relay did not answer in 15 s")), 15000);
    ws.onopen = () => ws.send(JSON.stringify(["REQ", "call", filter]));
    ws.onerror = (e) => { clearTimeout(timer); reject(new Error(`relay: ${e.message ?? e.type}`)); };
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg[0] === "EVENT" && msg[1] === "call") wraps.push(msg[2]);
      if (msg[0] === "EOSE") { clearTimeout(timer); ws.close(); resolve(); }
      if (msg[0] === "NOTICE" || msg[0] === "CLOSED") console.error("relay:", JSON.stringify(msg));
    };
  });
  if (!wraps.length) throw new Error(`no kind-1059 for ${to} on ${relay} in the last ${since} s`);
  // The relay lists by the time on the outside of the wrap, which is random
  // (up to two days back), so the first is not the one sent last. A call's
  // wrap is marked `call` and expires a short while after its making: of
  // the ones marked, the one that expires last was sent last. Without any
  // marked, the first listed.
  const expiry = (w) => Number((w.tags || []).find((t) => t[0] === "expiration")?.[1] ?? 0);
  const calls = wraps.filter((w) => (w.tags || []).some((t) => t[0] === "call"));
  const wrap = calls.length ? calls.sort((a, b) => expiry(b) - expiry(a))[0] : wraps[0];
  const call = calls.length > 0;
  console.error(`${wraps.length} wraps listed; wrap ${wrap.id.slice(0, 8)}… created_at=${wrap.created_at} tags=${JSON.stringify(wrap.tags)}${call ? " (a call)" : " (not marked as a call)"}`);
  return wrap;
}

/** An OAuth2 access token for FCM from the service account (RS256 JWT). */
async function accessToken(sa) {
  const now = Math.floor(Date.now() / 1000);
  const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
  const unsigned = `${b64({ alg: "RS256", typ: "JWT" })}.${b64({
    iss: sa.client_email,
    scope: "https://www.googleapis.com/auth/firebase.messaging",
    aud: sa.token_uri,
    iat: now,
    exp: now + 600,
  })}`;
  const signature = createSign("RSA-SHA256").update(unsigned).sign(sa.private_key, "base64url");
  const res = await fetch(sa.token_uri, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer", assertion: `${unsigned}.${signature}` }),
  });
  const body = await res.json();
  if (!res.ok) throw new Error(`token: ${res.status} ${JSON.stringify(body)}`);
  return body.access_token;
}

const wrap = eventFile ? JSON.parse(readFileSync(eventFile, "utf8")) : await newestWrap();
if (dry) {
  console.log(JSON.stringify(wrap));
  process.exit(0);
}
const sa = JSON.parse(readFileSync(account, "utf8"));
const trace = Math.random().toString(16).slice(2, 10);
const marked = (wrap.tags || []).some((t) => t[0] === "call");
const size = (data) => Object.entries(data).reduce((n, [k, v]) => n + k.length + v.length, 0);
// The two forms of the protocol: the event inside when the whole push fits
// in 3900 bytes, else its id and the relay to take it from (an invitation
// carries the SDP offer, some 20 KB: always the second form). `call` goes
// with either, as the new VPush marks it.
let data = { v: "2", type, event: JSON.stringify(wrap), trace, ...(marked ? { call: "1" } : {}) };
if (size(data) > 3900) {
  data = { v: "2", type, event_id: wrap.id, relay, trace, ...(marked ? { call: "1" } : {}) };
  console.error(`the event does not fit (${size({ event: JSON.stringify(wrap) })} bytes): sent by id and relay`);
}
const message = {
  message: {
    token,
    data,
    android: { priority: "HIGH", ttl: "60s", collapse_key: "call" },
  },
};
const bytes = size(data);
const bearer = await accessToken(sa);
const res = await fetch(`https://fcm.googleapis.com/v1/projects/${sa.project_id}/messages:send`, {
  method: "POST",
  headers: { authorization: `Bearer ${bearer}`, "content-type": "application/json" },
  body: JSON.stringify(message),
});
const body = await res.text();
console.log(`${res.status} ${body} (trace ${trace}, ${bytes} bytes)`);
process.exit(res.ok ? 0 : 1);
