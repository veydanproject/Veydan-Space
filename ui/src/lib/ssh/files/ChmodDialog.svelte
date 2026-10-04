<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Permission editor: a 3×3 rwx grid plus setuid/setgid/sticky, kept in sync
  with an octal field both ways. Works for local and remote (SFTP) entries.
-->
<script lang="ts">
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    open: boolean;
    /** File name(s) shown in the header. */
    targetLabel: string;
    /** Starting mode bits (permission bits only matter). */
    mode: number;
    onconfirm: (mode: number) => void;
    oncancel: () => void;
  }

  let { open = $bindable(), targetLabel, mode, onconfirm, oncancel }: Props = $props();

  // 9 permission bits (owner/group/other × r/w/x), highest bit first
  let bits = $state<boolean[]>(Array(9).fill(false));
  // special bits: setuid, setgid, sticky
  let special = $state<boolean[]>([false, false, false]);
  let octalText = $state('0000');

  $effect(() => {
    if (open) loadFromMode(mode);
  });

  function loadFromMode(m: number) {
    const perm = m & 0o777;
    bits = Array.from({ length: 9 }, (_, i) => (perm & (1 << (8 - i))) !== 0);
    special = [(m & 0o4000) !== 0, (m & 0o2000) !== 0, (m & 0o1000) !== 0];
    // Derive octal directly from m — reading bits/special here (via computeMode)
    // inside the open-effect would create a read-write cycle and loop the effect.
    octalText = format(m);
  }

  function computeMode(): number {
    let m = 0;
    for (let i = 0; i < 9; i++) if (bits[i]) m |= 1 << (8 - i);
    if (special[0]) m |= 0o4000;
    if (special[1]) m |= 0o2000;
    if (special[2]) m |= 0o1000;
    return m;
  }

  function format(m: number): string {
    return (m & 0o7777).toString(8).padStart(4, '0');
  }

  // Grid/checkbox change → recompute octal
  function onGridChange() {
    octalText = format(computeMode());
  }

  // Octal field change → repopulate grid
  function onOctalInput() {
    const clean = octalText.replace(/[^0-7]/g, '').slice(-4);
    const m = parseInt(clean || '0', 8);
    bits = Array.from({ length: 9 }, (_, i) => (m & (1 << (8 - i))) !== 0);
    special = [(m & 0o4000) !== 0, (m & 0o2000) !== 0, (m & 0o1000) !== 0];
  }

  const rows = [
    { key: 'owner', labelKey: 'files_chmod_owner' as const },
    { key: 'group', labelKey: 'files_chmod_group' as const },
    { key: 'other', labelKey: 'files_chmod_other' as const },
  ];
  const cols: { labelKey: 'files_chmod_read' | 'files_chmod_write' | 'files_chmod_exec' }[] = [
    { labelKey: 'files_chmod_read' },
    { labelKey: 'files_chmod_write' },
    { labelKey: 'files_chmod_exec' },
  ];

  function confirm() {
    onconfirm(computeMode());
  }
</script>

<Dialog {open} title={$t('files_chmod_title')} width="420px" onclose={oncancel}>
  <p class="chmod-target" title={targetLabel}>{targetLabel}</p>

  <table class="chmod-grid">
    <thead>
      <tr>
        <th></th>
        {#each cols as col (col.labelKey)}<th>{$t(col.labelKey)}</th>{/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row, r (row.key)}
        <tr>
          <td class="row-label">{$t(row.labelKey)}</td>
          {#each cols as _col, c (c)}
            <td>
              <input type="checkbox" bind:checked={bits[r * 3 + c]} onchange={onGridChange} />
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>

  <div class="special-row">
    <label><input type="checkbox" bind:checked={special[0]} onchange={onGridChange} /> setuid</label>
    <label><input type="checkbox" bind:checked={special[1]} onchange={onGridChange} /> setgid</label>
    <label><input type="checkbox" bind:checked={special[2]} onchange={onGridChange} /> sticky</label>
  </div>

  <div class="form-group octal-field">
    <label for="chmod-octal">{$t('files_chmod_octal')}</label>
    <input
      id="chmod-octal"
      type="text"
      inputmode="numeric"
      bind:value={octalText}
      oninput={onOctalInput}
      maxlength="4"
    />
  </div>

  {#snippet footer()}
    <button class="btn btn-ghost" onclick={oncancel}>{$t('cancel')}</button>
    <button class="btn btn-primary" onclick={confirm}>{$t('files_chmod_apply')}</button>
  {/snippet}
</Dialog>

<style>
  .chmod-target {
    color: var(--text-2);
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
    margin-bottom: var(--sp-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chmod-grid {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: var(--sp-3);
  }
  .chmod-grid th {
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    padding: var(--sp-1);
    text-align: center;
  }
  .chmod-grid td {
    padding: var(--sp-1);
    text-align: center;
  }
  .row-label {
    text-align: left !important;
    color: var(--text);
    font-size: var(--fs-sm);
  }

  .special-row {
    display: flex;
    gap: var(--sp-4);
    padding: var(--sp-2) 0;
    border-top: 1px solid var(--border);
    margin-bottom: var(--sp-3);
  }
  .special-row label {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    color: var(--text-2);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .special-row input[type='checkbox'] { flex-shrink: 0; }

  .octal-field input {
    font-family: var(--font-mono);
    max-width: 120px;
  }
</style>
