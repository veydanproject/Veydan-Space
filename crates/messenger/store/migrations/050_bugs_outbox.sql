-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Reports of a bug to the project (`messenger_runtime::bug`,
-- messenger-spec.md «Сообщение об ошибке») that did not reach the server
-- at once: the network failed, or the server answered with a failure of
-- its own. The body is kept as it is sent, byte for byte: the server knows
-- a body sent again by its reporter and `created_at` and answers the same
-- id. It is tried again with a growing pause, for seven days at most; the
-- shape is that of `msg_reports_outbox` (033).
--
--   id            local id of the queued report, 32 hex
--   body_json     the body as it goes (`POST /cloud/v1/bug`); '' once it
--                 is done, so that the words of the user do not stay
--   state         queued | sent | refused | expired
--   attempts      how many times it was tried, the first one counted
--   next_retry_at when it is tried next (unix seconds)
--   last_error    why the last try failed, or the word of a refusal
--
-- A row that is done stays thirty days, and goes then. Wiped with the
-- account, as it is signed by the identity.

CREATE TABLE IF NOT EXISTS msg_bugs_outbox (
    id            TEXT PRIMARY KEY NOT NULL,
    body_json     TEXT NOT NULL,
    state         TEXT NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    next_retry_at INTEGER NOT NULL,
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_bugs_outbox_due ON msg_bugs_outbox(state, next_retry_at);
