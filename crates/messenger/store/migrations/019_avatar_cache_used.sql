-- SPDX-FileCopyrightText: 2026 Veydan Project
-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
--
-- `msg_avatar_cache.used_at`: when the picture of the address was last
-- shown (at most once a day is written). Addresses nobody looked at for a
-- long time are forgotten, and their files with them, so the cache of
-- others' avatars does not grow for ever.

ALTER TABLE msg_avatar_cache ADD COLUMN used_at INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS msg_avatar_cache_sha ON msg_avatar_cache (sha256);
