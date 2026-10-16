-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Reports of a person to the moderators of the project
-- (`messenger_runtime::report`, messenger-spec.md «Жалоба») that did not
-- reach the server at once: the network failed, or the server answered
-- with a failure of its own. The report is kept as it is sent and tried
-- again with a growing pause, for seven days at most.
--
--   id            local id of the queued report, 32 hex
--   report_json   the report as it goes (messenger-evidence `Report`); ''
--                 once it is done, so that the keys of the messages it
--                 carries do not stay behind
--   state         queued | sent | refused | expired
--   attempts      how many times it was tried, the first one counted
--   next_retry_at when it is tried next (unix seconds)
--   last_error    why the last try failed, or the word of a refusal
--
-- A row that is done stays thirty days for the status of the device, and
-- goes then.

CREATE TABLE IF NOT EXISTS msg_reports_outbox (
    id            TEXT PRIMARY KEY NOT NULL,
    report_json   TEXT NOT NULL,
    state         TEXT NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    next_retry_at INTEGER NOT NULL,
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS msg_reports_outbox_due ON msg_reports_outbox(state, next_retry_at);
