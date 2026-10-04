-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- Relay API keys (`auth_type = 'api_key'`). The manifest ships the project
-- relay key inside the binary, so it is not a user secret; user-provided keys
-- for their own relays land here too. Never shown in the UI.

ALTER TABLE msg_relays ADD COLUMN auth_secret TEXT;
