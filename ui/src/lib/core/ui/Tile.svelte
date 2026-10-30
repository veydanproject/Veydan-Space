<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The square of an icon at the start of a row (36, `--radius`, the icon 18):
  `tone` colours it — `plain` a quiet one, `ok` / `warn` / `bad` a state,
  `accent` what waits or what the row is about, `clear` the icon alone in the
  accent, `dashed` the place of a new one.
-->
<script module lang="ts">
  export type TileTone = 'plain' | 'ok' | 'warn' | 'bad' | 'accent' | 'clear' | 'dashed';
</script>

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    icon: string;
    tone?: TileTone;
  }
  let { icon, tone = 'plain' }: Props = $props();
</script>

<span class="tile {tone}"><Icon name={icon} size={18} /></span>

<style>
  .tile {
    flex: none; display: grid; place-items: center; width: 36px; height: 36px; border-radius: var(--radius);
    background: var(--surface-3); color: var(--text-2);
  }
  .ok { background: var(--success-bg); color: var(--success-text, var(--success)); }
  .warn { background: var(--warn-bg); color: var(--warn-text); }
  .bad { background: var(--danger-bg); color: var(--danger-text); }
  .accent { background: var(--accent-tint); color: var(--accent-text); }
  .clear { background: transparent; color: var(--accent-text); }
  .dashed { background: none; color: inherit; border: 1px dashed var(--border-2); }
</style>
