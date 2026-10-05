-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- A message the user wrote is tried for an hour; then the outbox gives up
-- and the message is shown as not sent. A relay that refuses it outright is
-- a failure sooner, after a few refusals; a network that is down is not.
-- Rows that are not a message of the user (control events of groups,
-- deletions, edits) have no deadline and are tried until they leave.

ALTER TABLE msg_outbox ADD COLUMN expires_at INTEGER;
ALTER TABLE msg_outbox ADD COLUMN rejections INTEGER NOT NULL DEFAULT 0;

-- Visible messages queued under the old rules get their hour from when
-- they were queued.
UPDATE msg_outbox SET expires_at = created_at + 3600
 WHERE local_id IN (SELECT outbox_local_id FROM msg_messages
                     WHERE outbox_local_id IS NOT NULL AND is_hidden = 0);

UPDATE msg_outbox SET state = 'abandoned', last_error = 'expired: ' || COALESCE(last_error, 'not published')
 WHERE expires_at IS NOT NULL AND expires_at <= CAST(strftime('%s', 'now') AS INTEGER)
   AND state <> 'published';

-- Shown as failed after three attempts while the outbox kept trying: they
-- wait again, and leave or expire as any other.
UPDATE msg_messages SET status = 'queued', failure_reason = NULL
 WHERE status = 'failed' AND outbox_local_id IN
   (SELECT local_id FROM msg_outbox WHERE state IN ('queued', 'failed', 'publishing'));
