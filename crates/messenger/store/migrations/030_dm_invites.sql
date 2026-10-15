-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Invitations to write to me (a link, a code, a QR): one secret with its
-- term, its number of uses and, when it is for one person, their key. The
-- id is the sha256 of the secret, so my devices and the snapshot of my
-- relations name one invitation alike. The uses are a set of keys, not a
-- counter: two of my devices that each let somebody in by the same
-- invitation agree by union, where a maximum of two counters lost one.

CREATE TABLE msg_dm_invites (
  id         TEXT PRIMARY KEY,
  secret     TEXT NOT NULL,
  kind       TEXT NOT NULL,
  label      TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  expires_at INTEGER,
  max_uses   INTEGER,
  for_pubkey TEXT,
  revoked_at INTEGER
);

CREATE TABLE msg_dm_invite_uses (
  invite_id TEXT NOT NULL,
  peer      TEXT NOT NULL,
  at        INTEGER NOT NULL,
  PRIMARY KEY (invite_id, peer)
);
