-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Previews of pages outside, kept so that a page asked for once is not
-- asked again every time the chat is opened. A row appears only after
-- the user pressed the button; `preview_json` is the preview as the UI
-- gets it, picture included.

CREATE TABLE IF NOT EXISTS msg_link_previews (
    url          TEXT PRIMARY KEY NOT NULL,
    preview_json TEXT NOT NULL,
    fetched_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS msg_link_previews_age ON msg_link_previews (fetched_at);
