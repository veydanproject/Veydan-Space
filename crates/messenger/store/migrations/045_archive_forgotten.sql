-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- The scopes of the archive forgotten here (`archive::remove_scope_in`: a
-- private server removed by hand or by the snapshot of another device, a
-- group forgotten), until the handler of the sync entities takes them
-- (`archive::take_forgotten`): its positions of their segments live in
-- another database and are reset when it next runs.
CREATE TABLE IF NOT EXISTS msg_archive_forgotten (
    scope TEXT PRIMARY KEY,
    at    INTEGER NOT NULL
);
