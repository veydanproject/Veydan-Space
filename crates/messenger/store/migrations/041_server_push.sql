-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The push of a private server through the gateway (internal/messenger-wire.md,
-- «Push через шлюз»). A private server whose push mode is `veydan` runs a
-- push server of its own, which wakes the phone through the gateway of the
-- Veydan network.
--
--   push_url      the address of that push server, `https://…`; the card
--                 says it (`pu=`), the CLI puts it by hand; without it
--                 nothing is registered there
--   push_gateway  the gateway it named in its `/v1/info` (`gateway.url`),
--                 where the ticket of the device is taken
--   push_sender   the key it seals its pushes with (`gateway.sender`): a
--                 sealed push from any other key is dropped
--
-- `push_gateway` and `push_sender` are what this device heard, not what the
-- card said: they are not carried to my other devices, each asks itself.

ALTER TABLE msg_servers ADD COLUMN push_url TEXT;
ALTER TABLE msg_servers ADD COLUMN push_gateway TEXT;
ALTER TABLE msg_servers ADD COLUMN push_sender TEXT;
