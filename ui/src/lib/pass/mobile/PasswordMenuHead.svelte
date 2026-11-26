<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The head of a password's sheet of actions (DS2-31, DS2-17): its letter-avatar 36,
  as in its row (PasswordRow), and its title. Two pieces with no box of their own:
  the sheet's head (ContextMenu's .sheet-menu-header) lays them out.
-->
<script lang="ts">
  import { AVATAR_COLORS } from '$lib/core/mobile/nav-colors';
  import type { PasswordEntry } from '$lib/pass/types';

  let { entry }: { entry: PasswordEntry } = $props();

  function avatarColor(s: string): string {
    let h = 0;
    for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AVATAR_COLORS[h % AVATAR_COLORS.length];
  }
</script>

<span class="m-avatar" style:--c={avatarColor(entry.title)}>{entry.title.charAt(0).toUpperCase()}</span>
<span class="name">{entry.title}</span>

<style>
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
