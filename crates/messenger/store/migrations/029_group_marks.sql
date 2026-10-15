-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Marks of moderation in groups (`mod.hide`, `mod.unhide`, `mod.approve`,
-- `mod.reject`): a note names one or more messages (`target_id`, one row
-- per target, so that each row carries whether its own mark is in force)
-- or an author and a span of their dates (`author`, `from_at`, `to_at`).
-- Whether a mark is in force (`active`) is a function of the journal as
-- it stands now, recomputed whenever the journal changes; what the marks
-- say of a message is written into the message (`mod_state`, `review`).
-- `review`: `ok` (never held), `pending` (waits for a moderator),
-- `approved`, `rejected`.

CREATE TABLE msg_group_marks (
  note_id   TEXT NOT NULL,
  group_id  TEXT NOT NULL,
  kind      TEXT NOT NULL,
  target_id TEXT NOT NULL DEFAULT '',
  author    TEXT,
  from_at   INTEGER,
  to_at     INTEGER,
  version   TEXT,
  reason    TEXT NOT NULL DEFAULT '',
  by        TEXT NOT NULL,
  at        INTEGER NOT NULL,
  active    INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (note_id, target_id)
);
CREATE INDEX idx_group_marks_target ON msg_group_marks (group_id, target_id);
CREATE INDEX idx_group_marks_author ON msg_group_marks (group_id, author);

ALTER TABLE msg_messages ADD COLUMN mod_state TEXT NOT NULL DEFAULT '';
ALTER TABLE msg_messages ADD COLUMN review TEXT NOT NULL DEFAULT 'ok';
