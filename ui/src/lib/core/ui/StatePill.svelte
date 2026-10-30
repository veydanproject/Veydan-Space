<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The pill of a state at the end of a row or a head (22, `--fs-xs`): `ok` on its way, `warn` waiting, `bad` gone wrong. -->
<script module lang="ts">
  export type PillTone = 'ok' | 'warn' | 'bad';
</script>

<script lang="ts">
  interface Props {
    text: string;
    tone: PillTone;
  }
  let { text, tone }: Props = $props();
</script>

<span class="pill {tone}">{text}</span>

<style>
  .pill {
    flex: none; display: inline-flex; align-items: center; height: 22px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); white-space: nowrap;
    background: var(--success-bg); color: var(--success-text, var(--success));
  }
  .warn { background: var(--warn-bg); color: var(--warn-text); }
  .bad { background: var(--danger-bg); color: var(--danger-text); }
</style>
