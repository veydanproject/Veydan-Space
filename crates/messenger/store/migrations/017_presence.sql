-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Presence. A contact beats from a presence key of its own, derived from
-- its secret, and tells that key only to its approved contacts; a key it
-- stops telling (a contact removed or blocked) is replaced by a new one.
-- Everything is kept by its own key, so it holds whatever the order.
--
-- `msg_presence_keys`: the presence key each peer told me. The later
-- `since` wins; a peer that stopped sharing keeps a row with an empty
-- `presence_pubkey` and the `since` of the withdrawal, so a key told
-- before it and heard after it stays out.
--
-- `msg_presence`: the newest beat of a presence key. `seen_at` is the
-- beat's own time and only moves forward; the peer is online while now <
-- `online_until`. Only beats of a key some peer told me are kept, and a
-- key that is replaced or withdrawn takes its row with it.
--
-- `msg_presence_told`: the epoch of my own presence key each peer was last
-- told. A peer with another epoch, or none, is told again; a rotation
-- empties the table.

CREATE TABLE IF NOT EXISTS msg_presence_keys (peer TEXT PRIMARY KEY NOT NULL, presence_pubkey TEXT NOT NULL, since INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS msg_presence (presence_pubkey TEXT PRIMARY KEY NOT NULL, seen_at INTEGER NOT NULL, online_until INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS msg_presence_told (peer TEXT PRIMARY KEY NOT NULL, epoch INTEGER NOT NULL);
