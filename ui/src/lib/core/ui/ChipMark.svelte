<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  interface Props {
    kind: string;
    /**
     * The translated name of the kind ("Profile"). The letter is hidden from
     * a screen reader, so the caption is read before the chip's text instead.
     */
    caption?: string;
  }

  let { kind, caption }: Props = $props();

  const LETTER: Record<string, string> = {
    folder: 'f',
    workspace: 'w',
    profile: 'p',
    proxy: 'x',
    ssh: '>',
    totp: 'k',
    password: 'l',
    note: 'n',
    domain: 's',
    tag: 't',
  };

  const letter = $derived(LETTER[kind] ?? '');
</script>

{#if letter}
  <span class="mark" aria-hidden="true">{letter}</span>
{/if}
{#if caption}
  <span class="caption">{caption}: </span>
{/if}

<style>
  /* Theme color, not the chip color. */
  .mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 1px solid var(--text);
    color: var(--text);
    background: transparent;
    font-size: 8px;
    font-weight: 700;
    line-height: 1;
    flex-shrink: 0;
  }
  /* Read out, not shown. */
  .caption {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
</style>
