-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Corrects request_floor (022) to the rules of 2026-10-09 (022 itself is
-- left as it was: an applied migration must not change).
--
-- My own "Decline" no longer ends an episode: the peer I declined gets no
-- further message through. 022 and the code of that day raised the floor
-- for rows I declined that the peer has not answered since; the request I
-- declined counts again. Whatever floor came before my no lay behind that
-- request, so 0 decides the same.
UPDATE msg_dm_relations SET request_floor = 0
WHERE my_contact = 'declined' AND peer_signal = 'none';

-- A floor is never further ahead of the clock than FLOOR_SKEW_SECS (300 s):
-- a row dated in the future must stay above it and keep counting.
UPDATE msg_dm_relations
SET request_floor = CAST(strftime('%s', 'now') AS INTEGER) + 300
WHERE request_floor > CAST(strftime('%s', 'now') AS INTEGER) + 300;
