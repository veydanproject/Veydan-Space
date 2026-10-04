// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The placeholder and the hint of the "Tags and objects" picker name only what
 * this product can bind a note to (platform-spec 10.3): a tag and a folder
 * always, then each kind of entity the product has entities of — Notes alone
 * offers no proxies, SSH connections or TOTP entries.
 */

import { ENTITY_KINDS, type EntityKind } from '$lib/core/bindings';
import type { TranslationKey } from '$lib/core/i18n';

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

const KIND_KEY: Record<EntityKind, TranslationKey> = {
  workspace: 'notes_label_kind_workspace',
  profile: 'notes_label_kind_profile',
  proxy: 'notes_label_kind_proxy',
  ssh: 'notes_label_kind_ssh',
  totp: 'notes_label_kind_totp',
  password: 'notes_label_kind_password',
};

/** `kinds`: the entity kinds that have at least one entity here, in any order. */
export function labelPickerText(kinds: Iterable<string>, tr: Tr): { placeholder: string; hint: string } {
  const present = new Set(kinds);
  const entities = ENTITY_KINDS.filter((k) => present.has(k));
  const names = [tr('notes_label_kind_tag'), tr('notes_label_kind_folder'), ...entities.map((k) => tr(KIND_KEY[k]))];
  const list = names.join(', ');
  return {
    // The placeholder is short (a field cuts a long one); the hint under the field lists every kind.
    placeholder: entities.length > 0 ? tr('notes_label_placeholder_objects') : tr('notes_label_placeholder'),
    hint: tr('notes_label_hint', { kinds: list }),
  };
}
